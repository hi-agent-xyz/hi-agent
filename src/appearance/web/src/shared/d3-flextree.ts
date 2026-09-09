// d3-flextree — the tidy tree with genuinely variable node sizes: Buchheim's O(n)
// contour walk extended so a node's own width and height take part in the packing,
// rather than d3-hierarchy's uniform `nodeSize` with a `separation` callback
// standing in for it.
//
// Here for the same reason as `d3-hierarchy.ts` beside it, and not for a shared
// instance: it is pure functions over plain objects and holds no state. A view is
// transformed and never bundled (`views/mod.rs`), and there is no node_modules on a
// user's machine, so the only way a view can import a library at all is for the host
// to ship it and name it in the import map. See `LIBRARY_SPECIFIERS` in
// `vite.config.ts`.
//
// **Why this and not the tidy tree already shipped.** `factory/home` draws every open
// row as its own node, and a row is 320px wide and anywhere from 28 to 240px tall.
// d3-hierarchy's `tree()` takes one `nodeSize` for all of them; carrying the real
// heights in `separation` is an approximation that only holds while the tree is two
// levels deep, because `separation` sees a pair of siblings and never a subtree's
// contour. Measured on the live shape the two agree to within 2%, so this is not
// here for the pixels — it is here so that adding a third level (a sub-topic with
// rows of its own) does not silently start overlapping.
//
// It imports `d3-hierarchy` itself, which the import map already resolves, so this
// costs one more entry and no second copy of anything.
export * from "d3-flextree";
