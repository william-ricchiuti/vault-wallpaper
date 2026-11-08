# Local changes to glyphon 0.12.0

Source: crates.io `glyphon` 0.12.0 (MIT OR Apache-2.0 OR Zlib). Benches, examples and
dev-dependencies removed from `Cargo.toml`.

1. `src/text_render.rs`, `TextRenderer::prepare`: when the device has
   `Features::MAPPABLE_PRIMARY_BUFFERS`, the glyph vertices go into a new buffer created
   mapped with `VERTEX | MAP_WRITE` instead of `queue.write_buffer` into the old one.

Why: on Apple GPUs any copy command (which `write_buffer` and staged
`mapped_at_creation` use) wakes a ~72 MB driver pool for about a second, on top of
the ~100 MB render pool. Measured 2026-11-07, see `docs/plan.md` ("Redraw memory").
Atlas uploads (`queue.write_texture`) are unchanged: they happen only when new glyphs
are rasterised.
