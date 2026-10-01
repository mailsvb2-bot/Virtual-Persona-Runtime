# Self-hosted LiveKit client provenance

Owner Lab serves the LiveKit browser SDK from its own loopback origin. The executable artifact is generated from the exact npm package pinned by `package-lock.json`:

- package: `livekit-client`
- version: `2.22.3`
- upstream source: `livekit/client-sdk-js`, tag `v2.22.3`
- license: Apache-2.0
- generated runtime asset: `dist/vendor/livekit-client.umd.js`
- generated digest: `dist/vendor/livekit-client.umd.js.sha256`

`npm ci --ignore-scripts && npm run build` copies the UMD artifact from the locked npm package and regenerates its SHA-256 digest. CI requires the committed `dist/` tree to remain byte-for-byte current after that build, so a reviewed source/lock change is required to change the served SDK bytes.

No production Owner Lab page loads executable JavaScript from a third-party CDN.
