# Router deployment guide

Keep device-specific deployment records and backups outside the public repository.
Use an isolated test environment before changing a live device.

1. Preserve device settings, control intent, last-good configuration and a recovery copy of the installed Agent.
2. Build the target architecture in CI and verify the downloaded checksum.
3. Gracefully stop the Agent and its child core before replacing the executable.
4. Restore the intended running/stopped state and verify the controller and actual traffic.
5. Test a reversible local configuration update and restore it; confirm the running process and network state.

A device-local overlay can select `geosite-matcher: mph` when measured memory
limits make this appropriate. See `examples/router-low-memory.yaml`. Measure cold
startup and live validation alongside a running core; no fixed rule-count threshold
works for every device. Keep only necessary temporary files on RAM-backed storage.

Before enabling TUN on a gateway, account for client-side TUN, DNS interception and
fake-IP ranges. Verify representative LAN clients and their usual applications,
not only traffic originating on the gateway. Maintain an accessible recovery path.
