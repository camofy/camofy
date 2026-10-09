# Public CN ruleset mirror

This directory vendors the unchanged public classical CN export from
[MetaCubeX/meta-rules-dat](https://github.com/MetaCubeX/meta-rules-dat).

- Data revision: `ad2798bba7340c09298f364bebedebbfa4398f5b`
- [Original source](https://raw.githubusercontent.com/MetaCubeX/meta-rules-dat/ad2798bba7340c09298f364bebedebbfa4398f5b/geo/geosite/classical/cn.list)
- Original bytes: `2965298`
- Original SHA-256: `5a992276c844c69afad4bff79aeb7acd807249990d049e1373e54c24aecd0aed`
- Entries: `111224`, all `DOMAIN-SUFFIX`
- Storage: deterministic gzip, no filename and zero timestamp; decompression restores the exact source bytes.

The upstream project distributes its work under GPL-3.0. Its license is included
in `LICENSE`, retrieved from upstream source revision
`4178770badecb1b349fbcd62c737e0d7a2079729`. Original authors retain their rights.
This public dataset contains no Camofy identities, subscriptions or credentials.

The server embeds this snapshot and serves it at
`/api/rules/geosite/ad2798bba7340c09298f364bebedebbfa4398f5b/cn.list`.
It is not a proxy for arbitrary URLs and does not fetch data during a download.
Only the default, exact CN source with this content hash can use this mirror.

Future updates must use new versioned URLs and retain already published
snapshots. Never change the bytes behind an existing revision URL. Verify the
source digest, rules, attribution and conversion tests before updating data.
