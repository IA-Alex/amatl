# Chromium isolation boundary

Chromium remains an exceptional Deep capability and is disabled in AMATL's
runtime configuration. The verified isolation harness is
`packaging/amatl-chromium-sandbox`; it accepts an already downloaded local HTML
file, never a URL. This separation means `SafeFetcher` remains the only network
owner and Chromium cannot independently navigate, redirect or reach private
addresses.

The harness requires Linux, bubblewrap and a user systemd manager. It launches
Chromium inside new user, mount, PID, IPC, UTS, network and cgroup namespaces,
with an
empty root assembled from read-only `/usr`, `/opt` when needed, minimal devices,
the read-only Fontconfig configuration/cache, private `/tmp`, private profile
and no D-Bus sockets. The environment is emptied as well: `bwrap --clearenv`
drops everything the harness inherited, and only the harness's own `--setenv`
entries (private `HOME`/`TMPDIR`/XDG and dead D-Bus sockets) are restored, so
the service's bearer token and provider credentials are not reachable from
inside the sandbox. The network namespace is empty and is not connected to
the host or Internet. The transient
systemd unit enforces `MemoryMax`, `TasksMax` and `RuntimeMaxSec`; bubblewrap uses
`--die-with-parent`. Output is copied only after a successful exit and an 8 MiB
cap. The default memory ceiling is 1 GiB. The temporary profile and input copy
are deleted on every shell exit.
The host must permit unprivileged user namespaces; otherwise bubblewrap aborts
and the helper returns no DOM. The GitHub runner explicitly enables this kernel
primitive before the test because Ubuntu's AppArmor default restricts it.

The workflow pins `ubuntu-22.04` rather than `ubuntu-latest` for that reason.
GitHub migrated the `ubuntu-latest` label to the `ubuntu-26.04` image
(release `20261111.82`) and the gate went red on 2026-11-13, days later, with
nothing in this repository changed: on 26.04 the AppArmor policy denies the
namespace even after the sysctl is relaxed and the per-binary profile is
dropped. The pin makes moving the image a deliberate change someone reviews,
instead of a silent migration that turns the gate red for a reason no commit
explains. Revisit it when the isolation contract is known to hold on a newer
image.
CI downloads Chrome for Testing headless shell `152.0.7977.42` from Google's
versioned distribution URL and verifies SHA-256
`129686a270d84ac4637c614802c554634aa827aa13214216f81e0a0b9410f8cf` before use;
it does not depend on the runner's mutable Chromium installation.

The integration workflow proves four properties with real Chromium:

1. JavaScript transforms a local fixture and the final DOM is returned.
2. `fetch("http://127.0.0.1:...")` cannot reach a host listener because the
   browser has a separate empty network namespace.
3. timeout/error paths leave no running transient unit or reusable profile.
4. the browser's environment is empty: a stand-in Chromium that dumps `env`
   into the DOM shows a canary set for the harness is absent, while the
   harness's own `HOME` survives `--clearenv`.

This harness closes the OS isolation design and validation work; it is not yet
wired into `ChromiumRenderer`. Activation needs a bounded DOM-input renderer
API or a reviewed CDP bridge that preserves the same no-network ownership.
Passing a public URL directly to Chromium would violate the Fetcher/SSRF
contract and is prohibited.
