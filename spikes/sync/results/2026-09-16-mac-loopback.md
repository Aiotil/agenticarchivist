# Sync prototype run

Syncthing: vendor/syncthing/syncthing-macos-universal-v2.1.5/syncthing
Files: 30 × 4 MB

- PASS device ID computed from the certificate matches Syncthing's (6GYGCFV-FKRBKEB-VDMFQX7-3TTEIO6-YDOYGAN-RTVG4ME-FZSEFX2-4ADU3AW)
- PASS invite link is 544 characters
- PASS C's Syncthing runs with the identity derived from the invite secret, exactly as A predicted
- PASS C receives the collection from A (61 files identical)
  - 134 MB in 3.3 s (40 MB/s over loopback)
- PASS both devices' logs reach both devices (62 files identical)
- PASS A is fully up to date with C (62 files identical)
- PASS A and C merge to the same state (c46192c2de6b73fc)
- PASS concurrent title edits resolve to the later write: "1943 Letter to Rose"
- PASS edits to different fields are all kept
- PASS sidecars stay local to each device
- PASS no Syncthing conflict copies on A or C
- PASS C accepted B's seat from the signed add-member entry in A's log
  - A (the inviter) is now offline
- PASS B joins with A offline and receives the whole collection from C (62 files identical)
  - B synced in 28.2 s
- PASS B is connected to C only
- PASS B merges to the same state as C
- PASS B's role in the merged log is viewer
- PASS a viewer's own log entries are rejected when merging (title unchanged, no forged admin)
  - B's folder is receive-only, so Syncthing never sends B's log; merge-time checks are the second line of defence

Result: 0 failure(s)
