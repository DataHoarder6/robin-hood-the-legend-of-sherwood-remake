# Browser session directory, not browser gossip

The native multiplayer menu discovers games through serverless iroh-gossip over the Mainline DHT; the wasm client is relay-only and cannot join that swarm. A "server browser" on the web page therefore needs a small discovery service: a native host publishes a short-lived, signed listing (nickname, mission, content edition, invitation URL); the web page lists live listings and joins with the invitation, which remains the authority. The directory never hosts a game and never carries gameplay traffic. A link remains usable without the directory, so the directory is convenience, not a single point of failure.

**Consequences**: new service with retention and signing rules; the browser page gains a chooser that can list sessions without weakening the verified-invitation flow.
