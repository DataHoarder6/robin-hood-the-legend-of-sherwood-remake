# Invitations may be non-expiring

The host can issue an `rhmp3` invitation with no expiry. A time-limited ticket remains available; the default is unchanged. A non-expiring ticket stays cryptographically valid, but it does not keep a session alive — the host must still be running, and a closed session must be reported as ended. The ticket format must distinguish "never expires" from a timestamp without treating 0 as a valid deadline.

**Considered options**: extend the 30-minute window (still fails the owner's "connect at any time" need); reuse a redeemed ticket (already has separate semantics). **Consequences**: ticket schema bump and tests for issue/validate/reject paths.
