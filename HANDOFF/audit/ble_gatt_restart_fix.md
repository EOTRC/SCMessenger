# BLE GATT Server & Advertiser Restart Fix

## Stop Caller Identified
`stopMeshService()` (called during mesh service shutdown)

## Why Nothing Restarted It
The mesh service was stopped but never restarted (e.g., app went to background without restarting mesh service), so the startup path (`initializeAndStartBle()`) was never invoked. The prior code relied on manual restarts which never happened, leaving BLE down for 17 hours.

## What Was Changed
1. Added `isRunning`/`isAdvertising` flags to `BleGattServer` and `BleAdvertiser`
2. Implemented 5-minute BLE health check in `MeshRepository`
3. Added explicit log statements on every stop/start
4. Added health check restart path that kicks in when BLE is down

## Proving the Fix
On next device run, expect these log lines:
```
[WARN] BLE components not running, restarting due to health check
[OK] BLE components restarted by health check
```

**Note:** The health check runs *only* when the mesh service is active (e.g., app foreground), so these logs only appear when BLE was unexpectedly stopped.