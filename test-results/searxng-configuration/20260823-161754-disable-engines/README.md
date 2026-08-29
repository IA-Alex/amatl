Generated configuration artifact — SearXNG engine isolation — do not treat as project documentation.

# SearXNG engine isolation

- Timestamp: 2026-08-23T16:17:54-07:00
- Effective configuration: SearXNG merges `/usr/local/searxng/searx/settings.yml` with `/etc/searxng/settings.yml` because the latter sets `use_default_settings: true`.
- Modified file: `/etc/searxng/settings.yml` in container `searxng` (Docker volume `searxng-data`).
- Backup: `/etc/searxng/settings.yml.bak-20260823-161754-disable-engines` in that same configuration volume.
- Reload action: `docker restart searxng` only. The container returned to `running` at `2026-08-23T23:19:22Z`.

## Authorized delta

Only these configuration values changed:

| Engine | Before | After | Setting location |
| --- | --- | --- | --- |
| duckduckgo | `disabled: false` | `disabled: true` | lines 28-29 (line 29 changed) |
| mojeek | `disabled: false` | `disabled: true` | lines 37-38 (line 38 changed) |
| qwant | `disabled: false` | `disabled: true` | lines 39-40 (line 40 changed) |

No engine was enabled or added. No categories, network, proxy, timeout, rate-limit, AMATL, adapter, routing, ranking, canonicalization, deduplication, or Marginalia setting was changed.

## Effective validation after reload

The installed SearXNG loader reported `merge the default settings ( /usr/local/searxng/searx/settings.yml ) and the user settings ( /etc/searxng/settings.yml )` and resolved all target engines as `disabled: true`. They are absent from the effective enabled-engine inventory.

There are 111 enabled engines after the change. Engines with effective `general` or `web` category are: `wikipedia`, `wikidata`, `dogpile`, `google cse images`, `startpage news`, `startpage images`, `yandex api`, `wolframalpha_api`, `brave.images`, and `brave.videos`. Therefore SearXNG retains general-result sources. This reports existing effective defaults; it did not enable any of them.

See [effective-before.md](effective-before.md), [effective-after.md](effective-after.md), and [functional-test.md](functional-test.md).
