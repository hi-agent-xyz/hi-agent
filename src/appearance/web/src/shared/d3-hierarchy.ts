// d3-hierarchy — the tidy-tree layout (Reingold–Tilford via Buchheim), for any
// view that has to draw ownership rather than a list. Sessions is the first.
//
// Unlike its neighbours in `src/shared/`, this is NOT here for a shared instance:
// d3-hierarchy is pure functions over plain objects, holds no state, and two
// copies would behave identically. It is here because a view is *transformed*
// and never bundled (`views/mod.rs`: "No `--bundle`, so bare imports survive for
// the import map to resolve"), and there is no node_modules on a user's machine
// to bundle from — so the only way a view can import a library at all is for the
// host to ship it and name it in the map. See `LIBRARY_SPECIFIERS` in
// `vite.config.ts` for why that is a separate table from the shared-instance one.
export * from "d3-hierarchy";
