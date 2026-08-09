# P0 -- BLE L2CAP accept loop spins on terminal socket failure, destroying log observability

Status: Active
Severity: P0 (blocks the v0.4.0 five-node evidence gate)
Discovered: 2026-08-08 19:39 HST (Windows lane, live `adb logcat` pull)
Affects: Android, all builds carrying `BleL2capManager`
Source: `android/app/src/main/java/com/scmessenger/android/transport/ble/BleL2capManager.kt:86-97`
Evidence: `tmp/logs/pixel_full_20260808_1939.log` (gitignored, 508 MB, 3.8M lines)

## Summary

`BleL2capManager.startListening()` retries `BluetoothServerSocket.accept()` in a
tight loop with no backoff, no retry cap, and no socket re-creation. When the
server socket dies the failure is permanent, so the loop spins at CPU speed
logging a full stack trace on every iteration.

Measured on a Pixel 6a: **220,504 iterations in 37 minutes** (17:56:03 ->
18:33:00), roughly **99 iterations per second**, each emitting a 7+ frame stack
trace.

## The defect

```kotlin
// Accept loop
while (isListening) {
    try {
        val socket = serverSocket?.accept()
        if (socket != null) {
            handleIncomingConnection(socket)
        }
    } catch (e: Exception) {
        if (isListening) {
            Timber.e(e, "Error accepting L2CAP connection")
        }
    }
}
```

The caught exception is:

```
java.io.IOException: read failed, socket might closed or timeout, read ret: -1
    at android.bluetooth.BluetoothSocket.readAll(BluetoothSocket.java:1274)
    at android.bluetooth.BluetoothSocket.waitSocketSignal(BluetoothSocket.java:1208)
    at android.bluetooth.BluetoothSocket.accept(BluetoothSocket.java:885)
    at android.bluetooth.BluetoothServerSocket.accept(BluetoothServerSocket.java:253)
```

Three separate problems:

1. **No backoff.** The catch logs and the `while` re-enters `accept()`
   immediately.
2. **No terminal-failure detection.** `isListening` is never cleared in the
   inner catch, and `serverSocket` is never closed or re-created. A dead socket
   fails forever, and the code cannot tell a transient failure from a terminal
   one.
3. **A silent variant.** `serverSocket?.accept()` is a safe call. If
   `serverSocket` is null, `accept()` yields null, `socket != null` is false,
   and the loop spins with **no exception and no log at all** -- the same
   busy-loop with no evidence.

## Impact

1. **Android cannot accept inbound L2CAP connections.** This matches the
   operator's reported symptom exactly: *"BLE-only was functional but
   intermittent/unreliable, especially iOS -> Android."* iOS -> Android is the
   direction where Android must `accept()`. The listener socket is dead and is
   never rebuilt, so the receive path stays down for the life of the process.

2. **It destroys log observability for everything else.** 220k stack traces
   evicted the entire 13:35-13:55 field-test window from a 16 MiB ring buffer.
   The capture pulled at 19:39 only reaches back to **14:09**.

   This independently explains the 2026-08-08 observation that the app emitted
   only 151 app-owned lines out of 43,608, and that `MdnsServiceDiscovery` and
   `SubnetProbe` emitted **zero** lines "despite being wired with all
   permissions granted". They were most likely not silent -- they were
   **evicted**. That question was recorded as UNRESOLVED in
   `HANDOFF/ORCHESTRATOR_TAKEOVER_2026-08-08.md` section 6 item 7; this is a
   strong candidate answer.

3. **CPU and battery burn** at ~99 failing syscalls/sec on a mobile device.

4. **It blocks the five-node evidence gate.** The G1-G6 bundle requested on
   PR #139 -- per-fragment BLE/receipt correlation, ledger before/after counts,
   complete logs covering the whole window -- is **not collectable while this is
   live**. The storm evicts the evidence as it is produced. Running the two
   required runs before fixing this would waste both.

## Timeline from the capture

| Time (HST) | Event |
|---|---|
| 17:56:01 | app proc 30318 starts |
| 17:56:03 | accept-spin begins |
| 18:28:54 | operator turns Wi-Fi OFF (`setWifiEnabled ... enable=false`) |
| 18:29:22 | `com.google.android.bluetooth` (pid 16438) dies (`cch CEM`) |
| 18:29:24 | BluetoothAdapter re-initialised (new BT pid 4846) |
| 18:30:10 | app proc 1358 killed (`remove task`) |
| 18:30:12 | app proc 5157 starts |
| 18:30:54 | operator turns Wi-Fi back ON |
| 18:32:28 | BluetoothAdapter re-initialised again (new BT pid 5971) |
| 18:32:58 | app proc 5157 killed; 18:32:59 proc 6116 starts (current) |
| 18:33:00 | accept-spin ends (with the process that hosted it) |

Note the spin **began 33 minutes before** the Bluetooth stack death, so the BT
process death is not its cause. The spin spans multiple app processes, so it is
reproducible per-process rather than a one-off.

## Suggested fix

In the inner catch, distinguish terminal from transient and stop spinning:

- On `IOException` from `accept()`, close and null the `serverSocket`, then
  either re-create it under exponential backoff (capped, with a retry ceiling)
  or set `isListening = false` and surface the failure to `TransportManager`.
- Add a floor delay on any retry path so a fast-failing socket cannot busy-loop.
- Rate-limit or de-duplicate the log so one recurring failure cannot emit
  hundreds of thousands of stack traces.
- Handle the `serverSocket == null` case explicitly rather than letting the safe
  call spin silently.

## Verification

- Reproduce: start the app, kill/restart the Bluetooth stack, watch
  `adb logcat -s BleL2capManager` for sustained error output.
- After the fix, a dead L2CAP socket must produce a bounded number of log lines
  and either a working re-listen or an explicit surfaced failure.
- Re-pull logcat over a 30-minute window and confirm app-owned lines are
  dominated by real transport activity rather than one tag.

## Related

- `HANDOFF/ORCHESTRATOR_TAKEOVER_2026-08-08.md` section 6 item 7 (sparse Android
  logging, `MdnsServiceDiscovery`/`SubnetProbe` zero lines -- likely answered here)
- PR #143 (delivery tracing was pointed at a non-existent target; orthogonal, but
  its benefit is nullified while this storm evicts the buffer)
- PR #139 five-node evidence gate (blocked by this for evidence collection)
