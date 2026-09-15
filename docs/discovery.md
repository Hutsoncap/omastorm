# Discovery and launch follow-ups

Review preparation based on the
[September 15 write-up](https://github.com/wesleygrimes/omarchy-plugins/issues/1).
Website and GitHub topics have already been set by the maintainer. The site
refresh is local until approved; the items below are follow-ups, not completed
actions. Security fixes are being handled separately.

## Repository settings

Suggested description:

> Live NEXRAD weather radar for the Omarchy / Hyprland desktop, in your bar. Rust + QML.

After review, apply the description and enable Discussions:

```sh
gh repo edit wesleygrimes/omastorm --description 'Live NEXRAD weather radar for the Omarchy / Hyprland desktop, in your bar. Rust + QML.' --enable-discussions
```

Upload `preview.png` from the repository root under Settings → General →
Social preview. Review the crop before saving. These are remote settings,
so they are not applied by merging this branch.

## Website

- Review and deploy the site using [site/README.md](../site/README.md).
- Verify the canonical homepage, sharing image, robots.txt, and sitemap.xml
  on the deployed domain.
- Add/verify omastorm.com in Google Search Console and submit
  `https://omastorm.com/sitemap.xml`. Verification requires the domain owner's
  account and the verification value supplied by Google; no value is guessed
  or included in this branch.

## Marketplace

The separate security work owns the response-body fixes. Once it is merged
and verified, follow [RELEASING.md](RELEASING.md) to publish a release.
Coordinate a freeze of main during marketplace validation, and submit the
exact full commit SHA to
[the existing submission](https://github.com/omacom/omarchy-plugin-marketplace/issues/5970).
After acceptance, check the official catalog and Omarchy Archive for the
listing (Widgets category, weather tag).

## Community submission draft

Proposed entry for the Plugins section of awesome-omarchy, in alphabetical order:

```markdown
- [Omastorm](https://github.com/wesleygrimes/omastorm) - Live NEXRAD radar in the Omarchy bar.
```

Before submitting, read that repository's current contribution rules, check
that the entry is not already present, and run its required checks (the
write-up calls for `pre-commit run --all-files`). No external PR has been opened.

Suggested demo post copy, to tailor to each community's rules before posting:

> Omastorm puts live NEXRAD weather radar next to the clock in Omarchy.
> Expand it for a two-hour radar loop, search cities or stations, and switch
> between Pixels, Glyphs, and Stipple in your desktop theme. Built with Rust
> and QML. It's a beta for Omarchy 4, with NOAA NEXRAD coverage.
> Demo and install: https://omastorm.com
> Code and feedback: https://github.com/wesleygrimes/omastorm

The write-up suggests r/omarchy, r/unixporn, r/rust, and X. Select an appropriate
community and use the demo video; nothing has been posted.

## Separate product and release decisions

- International radar support belongs in
  [issue #38](https://github.com/wesleygrimes/omastorm/issues/38), with its own
  design and data-source work. The site describes current NEXRAD coverage.
- Consider roughly weekly releases with meaningful notes after the listing
  is stable; urgent fixes can still warrant an earlier release.
