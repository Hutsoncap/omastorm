//! Find the current generation of rotating Level II chunk directories.
//! S3 can retain keys from an earlier trip through volumes 1–999, while
//! expired directories leave holes in that ring. Every decision here uses
//! the timestamp in a chunk's name, never the directory number or the first
//! key returned by a listing.

use chrono::NaiveDateTime;
use nexrad_data::aws::realtime::{ChunkIdentifier, VolumeIndex, list_chunks_in_volume};
use nexrad_data::result::Result;
use std::future::Future;
use xml::reader::{EventReader, XmlEvent};

pub const LIST_LIMIT: usize = 1000;

/// The newest dated generation in one directory, with its chunks in order.
/// `accept` lets backfill exclude generations newer than the active scan and
/// lets a live transition exclude leftover chunks from a previous rotation.
pub fn generation(
    ids: Vec<ChunkIdentifier>,
    accept: impl Fn(NaiveDateTime) -> bool,
) -> Option<(NaiveDateTime, Vec<ChunkIdentifier>)> {
    let stamp = ids
        .iter()
        .map(|id| *id.date_time_prefix())
        .filter(|stamp| accept(*stamp))
        .max()?;
    let mut selected: Vec<_> = ids
        .into_iter()
        .filter(|id| *id.date_time_prefix() == stamp)
        .collect();
    selected.sort_by_key(ChunkIdentifier::sequence);
    Some((stamp, selected))
}

pub async fn list(site: &str, volume: VolumeIndex) -> Result<Vec<ChunkIdentifier>> {
    list_chunks_in_volume(site, volume, LIST_LIMIT).await
}

/// The delimiter rolls all keys in a volume up to one `CommonPrefixes` item,
/// so all occupied directories fit in one S3 page. This matters near the
/// rotation: expired directories are absent, and a binary search over 1–999
/// can mistake an old directory past a hole for the newest one.
fn parse_volumes(body: &str, site: &str) -> std::result::Result<Vec<VolumeIndex>, String> {
    let mut volumes = Vec::new();
    let mut in_common = false;
    let mut prefix = None::<String>;
    let mut truncated = None::<String>;
    let mut reading_truncated = false;
    for event in EventReader::new(body.as_bytes()) {
        match event.map_err(|e| format!("reading volume listing: {e}"))? {
            XmlEvent::StartElement { name, .. } => match name.local_name.as_str() {
                "CommonPrefixes" => in_common = true,
                "Prefix" if in_common => prefix = Some(String::new()),
                "IsTruncated" => {
                    truncated = Some(String::new());
                    reading_truncated = true;
                }
                _ => {}
            },
            XmlEvent::Characters(value) => {
                if let Some(prefix) = prefix.as_mut() {
                    prefix.push_str(&value);
                } else if reading_truncated {
                    truncated.as_mut().unwrap().push_str(&value);
                }
            }
            XmlEvent::EndElement { name } => match name.local_name.as_str() {
                "Prefix" if in_common => {
                    if let Some(value) = prefix.take()
                        && let Some(number) = value
                            .strip_prefix(site)
                            .and_then(|value| value.strip_prefix('/'))
                            .and_then(|value| value.strip_suffix('/'))
                            .and_then(|value| value.parse::<usize>().ok())
                        && (1..=999).contains(&number)
                    {
                        volumes.push(VolumeIndex::new(number));
                    }
                }
                "CommonPrefixes" => in_common = false,
                "IsTruncated" => reading_truncated = false,
                _ => {}
            },
            _ => {}
        }
    }
    if truncated.as_deref() != Some("false") {
        return Err("volume listing is truncated or missing IsTruncated".into());
    }
    volumes.sort_unstable();
    volumes.dedup();
    Ok(volumes)
}

async fn occupied_volumes(site: &str) -> std::result::Result<Vec<VolumeIndex>, String> {
    let url = format!(
        "https://unidata-nexrad-level2-chunks.s3.amazonaws.com?list-type=2&prefix={site}/&delimiter=/&max-keys=1000"
    );
    let body = reqwest::get(url)
        .await
        .map_err(|e| format!("listing volumes: {e}"))?
        .error_for_status()
        .map_err(|e| format!("listing volumes: {e}"))?
        .text()
        .await
        .map_err(|e| format!("reading volume listing: {e}"))?;
    parse_volumes(&body, site)
}

/// Search occupied directories by each one's *newest* scan time. The
/// upstream iterator samples its first listed object's upload time; when a
/// directory contains old and new generations that sample can land many
/// hours behind live. Sorting occupied directory numbers preserves the ring
/// order without treating expired directories as search positions.
pub async fn latest(
    site: &str,
) -> std::result::Result<Option<(VolumeIndex, NaiveDateTime, Vec<ChunkIdentifier>)>, String> {
    let volumes = occupied_volumes(site).await?;
    let index = search(volumes.len(), |index| {
        let volume = volumes[index];
        async move {
            generation(list(site, volume).await.map_err(|e| e.to_string())?, |_| {
                true
            })
            .map(|(stamp, _)| stamp)
            .ok_or_else(|| format!("volume {} emptied during discovery", volume.as_number()))
        }
    })
    .await?;
    let Some(index) = index else { return Ok(None) };
    let volume = volumes[index];
    Ok(
        generation(list(site, volume).await.map_err(|e| e.to_string())?, |_| {
            true
        })
        .map(|(stamp, ids)| (volume, stamp, ids)),
    )
}

/// In occupied-directory order, scan times increase until the ring wraps
/// back to old retained data. Comparing each midpoint with the first time
/// locates the last directory before that wrap.
async fn search<F, V, E>(
    count: usize,
    mut probe: impl FnMut(usize) -> F,
) -> std::result::Result<Option<usize>, E>
where
    F: Future<Output = std::result::Result<V, E>>,
    V: PartialOrd,
{
    if count == 0 {
        return Ok(None);
    }
    let first = probe(0).await?;
    let mut low = 0;
    let mut high = count;
    while low < high {
        let mid = low + (high - low) / 2;
        let value = probe(mid).await?;
        if value >= first {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    Ok(Some(low - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;
    use nexrad_data::aws::realtime::ChunkIdentifier;

    fn stamp(day: u32, hour: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 9, day)
            .unwrap()
            .and_hms_opt(hour, 0, 0)
            .unwrap()
    }

    fn id(volume: usize, day: u32, hour: u32, sequence: usize) -> ChunkIdentifier {
        let name = format!("202609{day:02}-{hour:02}0000-{sequence:03}-I");
        ChunkIdentifier::from_name("KJAX".into(), VolumeIndex::new(volume), name, None).unwrap()
    }

    #[test]
    fn mixed_volume_selects_only_its_newest_generation() {
        let mixed = vec![id(130, 9, 5, 1), id(130, 12, 17, 2), id(130, 12, 17, 1)];
        let selected = generation(mixed.clone(), |_| true).unwrap();
        assert_eq!(selected.0, stamp(12, 17));
        assert_eq!(
            selected
                .1
                .iter()
                .map(ChunkIdentifier::sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
        let backfill = generation(mixed, |time| time < stamp(12, 17)).unwrap();
        assert_eq!(backfill.0, stamp(9, 5));
        assert_eq!(backfill.1.len(), 1);
    }

    #[test]
    fn volume_listing_omits_expired_directories_and_rejects_truncation() {
        let body = r#"<ListBucketResult><Prefix>KFCX/</Prefix><IsTruncated>false</IsTruncated>
            <CommonPrefixes><Prefix>KFCX/999/</Prefix></CommonPrefixes>
            <CommonPrefixes><Prefix>KFCX/69/</Prefix></CommonPrefixes>
            <CommonPrefixes><Prefix>KFCX/0/</Prefix></CommonPrefixes>
            <CommonPrefixes><Prefix>KFCX/1/</Prefix></CommonPrefixes>
            </ListBucketResult>"#;
        assert_eq!(
            parse_volumes(body, "KFCX")
                .unwrap()
                .iter()
                .map(VolumeIndex::as_number)
                .collect::<Vec<_>>(),
            vec![1, 69, 999]
        );
        assert!(parse_volumes(&body.replace("false", "true"), "KFCX").is_err());
    }

    #[test]
    fn sparse_ring_finds_live_volume_before_expired_hole() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                // KFCX's live branch ended at 68. Volume 69 still held a
                // September 9 generation, 70..75 had expired, and 999 held
                // a scan from earlier today. Search only occupied positions.
                let occupied = [
                    (1, 120_001),
                    (68, 120_068),
                    (69, 90_069),
                    (76, 90_076),
                    (999, 119_999),
                ];
                let found = search(occupied.len(), |index| async move {
                    Ok::<_, ()>(occupied[index].1)
                })
                .await
                .unwrap();
                assert_eq!(found.map(|index| occupied[index].0), Some(68));

                // With hundreds of occupied directories, the old search
                // returned the first old volume after the wrap (83), not 81.
                let occupied = (1..=81)
                    .map(|number| (number, 120_000 + number))
                    .chain((83..=838).map(|number| (number, 90_000 + number)))
                    .chain([(999, 119_999)])
                    .collect::<Vec<_>>();
                let found = search(occupied.len(), |index| {
                    let stamp = occupied[index].1;
                    async move { Ok::<_, ()>(stamp) }
                })
                .await
                .unwrap();
                assert_eq!(found.map(|index| occupied[index].0), Some(81));
            });
    }

    #[test]
    fn search_finds_today_before_old_cycle_even_with_reused_folders() {
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                // Current scans have wrapped to volumes 1..130; 131..999 retain the
                // previous cycle. Volume 999 must be included in the search.
                let found = search(999, |index| async move {
                    Ok::<_, ()>(if index < 130 {
                        10_000 + index as i32
                    } else {
                        index as i32
                    })
                })
                .await
                .unwrap();
                assert_eq!(found, Some(129));
                let found = search(999, |index| async move { Ok::<_, ()>(index as i32) })
                    .await
                    .unwrap();
                assert_eq!(found, Some(998));
            });
    }
}
