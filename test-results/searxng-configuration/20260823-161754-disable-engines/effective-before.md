Generated configuration artifact — SearXNG engine isolation — do not treat as project documentation.

# Precheck

Effective configuration was unambiguous: the running `searxng/searxng:latest` container mounts the `searxng-data` Docker volume at `/etc/searxng`; its active process loads `/etc/searxng/settings.yml` and merges it with the installed defaults.

Relevant user override before the change (secret fields omitted):

```yaml
use_default_settings: true
engines:
  - name: duckduckgo
    disabled: false
  - name: mojeek
    disabled: false
  - name: qwant
    disabled: false
```

All three were effective enabled engines. `mojeek` and `qwant` had categories `general, web`; DuckDuckGo participates in the default general search selection. The enabled inventory also contained general/web engines other than these three, so the change would not make SearXNG empty for general search.

Exact affected parameters: `/etc/searxng/settings.yml` lines 29, 38, and 40: each was `disabled: false` under the named engine. A backup was created before editing at `/etc/searxng/settings.yml.bak-20260823-161754-disable-engines`.
