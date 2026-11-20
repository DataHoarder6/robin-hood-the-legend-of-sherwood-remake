# Mission-scoped browser content, cached by digest

The browser must never be made to download the whole 1.1 GB retail DATA. The proposed unit is a mission bundle: shared core files plus exactly the files one mission loads, each hashed in a catalog. The invitation names the bundle's content identity; the browser downloads only that bundle, verifies every file, and stores it in the Cache API under a versioned name keyed by the digest. A second join or a replay of the same mission reads verified bytes locally; a different mission downloads its own bundle. Cache entries are advisory ( browsers may evict them), so a miss always re-fetches from the origin.

**Status**: accepted as the design direction; not yet implemented. **Consequences**: requires a converter that resolves per-mission dependencies, a content catalog, ticket/mission binding, and browser cache tests. The current Demo single-blob path stays until the new path is proved.
