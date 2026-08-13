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
private `/tmp`, private profile and no D-Bus sockets. The network namespace is
empty and is not connected to the host or Internet. The transient
systemd unit enforces `MemoryMax`, `TasksMax` and `RuntimeMaxSec`; bubblewrap uses
`--die-with-parent`. Output is copied only after a successful exit and an 8 MiB
cap. The default memory ceiling is 1 GiB. The temporary profile and input copy
are deleted on every shell exit.
The host must permit unprivileged user namespaces; otherwise bubblewrap aborts
and the helper returns no DOM. The GitHub runner explicitly enables this kernel
primitive before the test because Ubuntu's AppArmor default restricts it.

The integration workflow proves three properties with real Chromium:

1. JavaScript transforms a local fixture and the final DOM is returned.
2. `fetch("http://127.0.0.1:...")` cannot reach a host listener because the
   browser has a separate empty network namespace.
3. timeout/error paths leave no running transient unit or reusable profile.

This harness closes the OS isolation design and validation work; it is not yet
wired into `ChromiumRenderer`. Activation needs a bounded DOM-input renderer
API or a reviewed CDP bridge that preserves the same no-network ownership.
Passing a public URL directly to Chromium would violate the Fetcher/SSRF
contract and is prohibited.
