# Browser is a guest, never a host

Browser wasm builds cannot host a session by construction. The first web page therefore offers two explicit choices — play the Demo mission alone, or join a friend's game — and a shared invitation URL skips the chooser into a verified join. A native process (on a player's machine or on an estate host) is always the authority. A session that has ended always says so; it never leaves a guest waiting for a link to become valid.

**Consequences**: no browser-side matchmaking or authority code; "any time" means mid-session join while the host runs, not after shutdown.
