# Robin Hood port

The vocabulary for playing this game on two machines, where one may be a browser.

## Language

**Native host**:
The player process that owns the session and simulates the authoritative world.
_Avoid_: server, dedicated server — it is a game process, not a service.

**Browser guest**:
A browser that joins a hosted session; it cannot create or host one.
_Avoid_: browser host, web player that hosts.

**Invitation**:
A host-signed `rhmp3` ticket naming the host, relay, content edition and native content identity. It may be time-limited or non-expiring.
_Avoid_: link (the URL merely carries it), endpoint ID (native-only join).

**Content edition**:
The package family a guest must load to match the host: Demo or Full.
_Avoid_: datadir, install.

**Mission bundle**:
A content-addressed set of exactly the files one mission needs, plus the shared core files. It is the proposed unit of browser download; not yet implemented.
_Avoid_: datadir (the whole closure), package (the whole edition).

**Content identity**:
The digest in the invitation that binds the guest's bytes to the host's.
_Avoid_: version, commit — those identify the engine, not the data.
