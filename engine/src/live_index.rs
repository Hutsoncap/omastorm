//! Find the current generation of rotating Level II chunk directories.
//! S3 can retain keys from an earlier trip through volumes 1–999, while
//! expired directories leave holes in that ring. Every decision here uses
//! the timestamp in a chunk's name. Join lists occupied directories, then
//! picks the generation with the newest name timestamp. Ring position
//! cannot win. Leftover directories are not waited out once a live stamp
//! is already in hand.

use chrono::{NaiveDateTime, TimeDelta, Utc};
use nexrad_data::aws::realtime::{ChunkIdentifier, VolumeIndex, list_chunks_in_volume};
use nexrad_data::result::Result;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use xml::reader::{EventReader, XmlEvent};

pub const LIST_LIMIT: usize = 1000;
/// How many volume listings run at once when finding the newest stamp.
const PROBE_CAP: usize = 64;
/// A name timestamp this fresh is the live volume. Leftover directories
/// from an earlier trip around the ring cannot beat it, so join stops
/// asking them. Tight enough that the previous scan (several minutes old)
/// does not win the race.
const LIVE_AGE: TimeDelta = TimeDelta::seconds(90);
/// Accept a name a few seconds ahead of the local clock.
const CLOCK_SKEW: TimeDelta = TimeDelta::seconds(30);
/// Probes ignore keys older than this. Leftover-only directories come back
/// empty instead of listing a thousand stale objects. A station silent
/// longer than this reports no volume, which matches unavailable.
const RECENT: TimeDelta = TimeDelta::hours(3);
/// Once any recent volume is found, only these neighbors are checked for a
/// newer one. That is the current cycle, not 900 leftover folders.
const CLUSTER_RADIUS: usize = 48;

/// True when `stamp` is the live scan: no older than `LIVE_AGE`, and not
/// far in the future.
fn stamp_is_live(stamp: NaiveDateTime, now: NaiveDateTime) -> bool {
    let age = now - stamp;
    age <= LIVE_AGE && age >= -CLOCK_SKEW
}

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

/// Among every listed chunk, the volume whose newest generation is latest.
/// Leftover directories inside the current cycle cannot win on ring position.
#[cfg_attr(not(test), allow(dead_code))]
pub fn latest_from_chunks(
    ids: Vec<ChunkIdentifier>,
) -> Option<(VolumeIndex, NaiveDateTime, Vec<ChunkIdentifier>)> {
    let mut by_volume: HashMap<usize, Vec<ChunkIdentifier>> = HashMap::new();
    for id in ids {
        by_volume
            .entry(id.volume().as_number())
            .or_default()
            .push(id);
    }
    let mut best = None;
    for (number, ids) in by_volume {
        let Some((stamp, ids)) = generation(ids, |_| true) else {
            continue;
        };
        match &best {
            Some((best_stamp, _, _)) if stamp <= *best_stamp => {}
            _ => best = Some((stamp, VolumeIndex::new(number), ids)),
        }
    }
    best.map(|(stamp, volume, ids)| (volume, stamp, ids))
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

fn gcd(mut a: usize, mut b: usize) -> usize {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Visit every occupied directory, but spread the first wave across the
/// ring so today's cluster is not sitting behind folder 1.
fn spread_ring(volumes: Vec<VolumeIndex>) -> Vec<VolumeIndex> {
    let n = volumes.len();
    if n < 2 {
        return volumes;
    }
    let mut stride = (n / PROBE_CAP).max(1);
    while gcd(stride, n) != 1 {
        stride += 1;
        if stride >= n {
            return volumes;
        }
    }
    // Start at `stride`, not 0, so folder 1 is not the first probe.
    (0..n).map(|i| volumes[((i + 1) * stride) % n]).collect()
}

fn neighbor_volumes(occupied: &HashSet<usize>, center: usize, radius: usize) -> Vec<VolumeIndex> {
    let mut out = Vec::new();
    for d in -(radius as i32)..=radius as i32 {
        let mut n = center as i32 + d;
        if n < 1 {
            n += 999;
        } else if n > 999 {
            n -= 999;
        }
        let n = n as usize;
        if occupied.contains(&n) {
            out.push(VolumeIndex::new(n));
        }
    }
    out
}

type Found = (NaiveDateTime, VolumeIndex, Vec<ChunkIdentifier>);

fn keep_newest(best: &mut Option<Found>, got: Found) {
    match best {
        Some((stamp, _, _)) if got.0 <= *stamp => {}
        _ => *best = Some(got),
    }
}

async fn probe_volumes(
    site: &str,
    volumes: Vec<VolumeIndex>,
    after: NaiveDateTime,
    stop: impl Fn(&Found) -> bool,
) -> Option<Found> {
    if volumes.is_empty() {
        return None;
    }
    let sem = Arc::new(Semaphore::new(PROBE_CAP));
    let mut set = JoinSet::new();
    let site = site.to_owned();
    for volume in volumes {
        let site = site.clone();
        let sem = sem.clone();
        set.spawn(async move {
            let _permit = sem.acquire_owned().await.ok()?;
            let ids = list_after(&site, volume, after).await.ok()?;
            generation(ids, |_| true).map(|(stamp, ids)| (stamp, volume, ids))
        });
    }
    let mut best = None;
    while let Some(joined) = set.join_next().await {
        if let Some(got) = joined.ok().flatten() {
            keep_newest(&mut best, got);
            if best.as_ref().is_some_and(&stop) {
                set.abort_all();
                break;
            }
        }
    }
    best
}

fn parse_chunk_keys(
    body: &str,
    site: &str,
    volume: VolumeIndex,
) -> std::result::Result<Vec<ChunkIdentifier>, String> {
    let mut ids = Vec::new();
    let mut in_key = false;
    let mut key = None::<String>;
    for event in EventReader::new(body.as_bytes()) {
        match event.map_err(|e| format!("reading chunk listing: {e}"))? {
            XmlEvent::StartElement { name, .. } if name.local_name == "Key" => {
                in_key = true;
                key = Some(String::new());
            }
            XmlEvent::Characters(value) if in_key => {
                if let Some(key) = key.as_mut() {
                    key.push_str(&value);
                }
            }
            XmlEvent::EndElement { name } if name.local_name == "Key" => {
                in_key = false;
                if let Some(value) = key.take() {
                    let name = value.rsplit('/').next().unwrap_or(&value);
                    if let Ok(id) =
                        ChunkIdentifier::from_name(site.to_owned(), volume, name.to_owned(), None)
                    {
                        ids.push(id);
                    }
                }
            }
            _ => {}
        }
    }
    Ok(ids)
}

async fn list_after(
    site: &str,
    volume: VolumeIndex,
    after: NaiveDateTime,
) -> std::result::Result<Vec<ChunkIdentifier>, String> {
    let prefix = format!("{}/{}/", site, volume.as_number());
    let start = format!("{prefix}{}", after.format("%Y%m%d-%H%M%S"));
    let url = format!(
        "https://unidata-nexrad-level2-chunks.s3.amazonaws.com?list-type=2&prefix={prefix}&start-after={start}&max-keys={LIST_LIMIT}"
    );
    let body = reqwest::get(url)
        .await
        .map_err(|e| format!("listing volume {}: {e}", volume.as_number()))?
        .error_for_status()
        .map_err(|e| format!("listing volume {}: {e}", volume.as_number()))?
        .text()
        .await
        .map_err(|e| format!("reading volume {}: {e}", volume.as_number()))?;
    parse_chunk_keys(&body, site, volume)
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

/// The newest dated generation on the station, with its chunks in order.
/// Occupied directories are listed once. One spread wave looks for any
/// recent name; neighbors of that hit are the current cycle. Leftover
/// folders are not listed. Backfill still walks earlier volumes after join.
pub async fn latest(
    site: &str,
) -> std::result::Result<Option<(VolumeIndex, NaiveDateTime, Vec<ChunkIdentifier>)>, String> {
    let volumes = occupied_volumes(site).await?;
    if volumes.is_empty() {
        return Ok(None);
    }
    let now = Utc::now().naive_utc();
    let after = now - RECENT;
    let occupied: HashSet<usize> = volumes.iter().map(VolumeIndex::as_number).collect();
    let ordered = spread_ring(volumes);
    // Chunks expire in about an hour, so the live run may be only ~8
    // folders wide. Sample tightly enough that the first wave cannot
    // step over it.
    let sample = ordered.len().div_ceil(6).max(PROBE_CAP);
    let first: Vec<_> = ordered.iter().copied().take(sample).collect();
    let rest: Vec<_> = ordered.iter().copied().skip(sample).collect();
    let mut best = probe_volumes(site, first, after, |found| now - found.0 <= RECENT).await;
    if best
        .as_ref()
        .is_some_and(|(stamp, _, _)| stamp_is_live(*stamp, now))
    {
        return Ok(best.map(|(stamp, volume, ids)| (volume, stamp, ids)));
    }
    if let Some((stamp, volume, _)) = &best
        && now - *stamp <= RECENT
    {
        if let Some(got) = probe_volumes(
            site,
            neighbor_volumes(&occupied, volume.as_number(), CLUSTER_RADIUS),
            after,
            |found| stamp_is_live(found.0, now),
        )
        .await
        {
            keep_newest(&mut best, got);
        }
        return Ok(best.map(|(stamp, volume, ids)| (volume, stamp, ids)));
    }
    if let Some(got) = probe_volumes(site, rest, after, |found| stamp_is_live(found.0, now)).await {
        keep_newest(&mut best, got);
    }
    Ok(best.map(|(stamp, volume, ids)| (volume, stamp, ids)))
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

    /// The previous join search: occupied-directory order, times increase
    /// until one wrap. Leftover directories in the middle break it.
    async fn ring_search<F, V, E>(
        count: usize,
        mut probe: impl FnMut(usize) -> F,
    ) -> std::result::Result<Option<usize>, E>
    where
        F: std::future::Future<Output = std::result::Result<V, E>>,
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
    fn leftover_directories_inside_the_cycle_lose_to_the_newest_stamp() {
        let chunks = vec![
            id(1, 1, 5, 1),
            id(2, 12, 17, 1),
            id(2, 12, 17, 2),
            id(3, 2, 8, 1),
            id(4, 3, 8, 1),
        ];
        let stamps = [stamp(1, 5), stamp(12, 17), stamp(2, 8), stamp(3, 8)];
        tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap()
            .block_on(async {
                let found = ring_search(
                    stamps.len(),
                    |index| async move { Ok::<_, ()>(stamps[index]) },
                )
                .await
                .unwrap();
                assert_eq!(found, Some(3), "the old search picks the leftover at 4");
            });
        let (volume, time, ids) = latest_from_chunks(chunks).unwrap();
        assert_eq!(volume.as_number(), 2);
        assert_eq!(time, stamp(12, 17));
        assert_eq!(
            ids.iter()
                .map(ChunkIdentifier::sequence)
                .collect::<Vec<_>>(),
            vec![1, 2]
        );
    }

    #[test]
    fn newest_stamp_wins_on_a_sparse_ring_and_a_wrap() {
        let chunks = vec![
            id(1, 12, 1, 1),
            id(68, 12, 18, 1),
            id(69, 9, 5, 1),
            id(76, 9, 6, 1),
            id(999, 12, 0, 1),
        ];
        let (volume, time, _) = latest_from_chunks(chunks).unwrap();
        assert_eq!(volume.as_number(), 68);
        assert_eq!(time, stamp(12, 18));

        let wrapped = vec![
            id(1, 12, 20, 1),
            id(2, 12, 22, 1),
            id(200, 11, 4, 1),
            id(999, 12, 16, 1),
        ];
        let (volume, time, _) = latest_from_chunks(wrapped).unwrap();
        assert_eq!(volume.as_number(), 2);
        assert_eq!(time, stamp(12, 22));
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
    fn neighbors_wrap_around_the_ring() {
        let occupied: HashSet<usize> = [1, 2, 998, 999].into_iter().collect();
        let near = neighbor_volumes(&occupied, 1, 2);
        let nums: Vec<_> = near.iter().map(VolumeIndex::as_number).collect();
        assert!(nums.contains(&1) && nums.contains(&2));
        assert!(nums.contains(&998) && nums.contains(&999));
    }

    #[test]
    fn spread_ring_visits_every_directory_and_leaves_the_first_wave_off_folder_one() {
        let volumes: Vec<_> = (1..=20).map(VolumeIndex::new).collect();
        let spread = spread_ring(volumes);
        let mut seen: Vec<_> = spread.iter().map(VolumeIndex::as_number).collect();
        assert_eq!(seen.len(), 20);
        seen.sort_unstable();
        assert_eq!(seen, (1..=20).collect::<Vec<_>>());
        assert_ne!(
            spread[0].as_number(),
            1,
            "the first probe must not always be folder 1"
        );
    }

    #[test]
    fn chunk_listing_keeps_only_keys_in_the_volume() {
        let body = r#"<ListBucketResult>
            <Contents><Key>KFCX/305/20260913-111000-001-S</Key></Contents>
            <Contents><Key>KFCX/305/20260913-141335-001-S</Key></Contents>
            <Contents><Key>KFCX/305/20260913-141335-002-I</Key></Contents>
            </ListBucketResult>"#;
        let ids = parse_chunk_keys(body, "KFCX", VolumeIndex::new(305)).unwrap();
        assert_eq!(
            ids.iter().map(ChunkIdentifier::name).collect::<Vec<_>>(),
            vec![
                "20260913-111000-001-S",
                "20260913-141335-001-S",
                "20260913-141335-002-I"
            ]
        );
    }

    #[test]
    fn a_stamp_from_the_last_minute_is_live() {
        let now = NaiveDate::from_ymd_opt(2026, 9, 13)
            .unwrap()
            .and_hms_opt(14, 0, 0)
            .unwrap();
        assert!(stamp_is_live(now, now));
        assert!(stamp_is_live(now - TimeDelta::seconds(30), now));
        assert!(stamp_is_live(now - TimeDelta::seconds(90), now));
        assert!(stamp_is_live(now + TimeDelta::seconds(10), now));
        assert!(!stamp_is_live(now - TimeDelta::seconds(91), now));
        assert!(!stamp_is_live(now - TimeDelta::minutes(8), now));
        assert!(!stamp_is_live(now + TimeDelta::seconds(31), now));
    }
}
