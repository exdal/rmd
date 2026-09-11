# Example environment

A minimal DM environment for exercising the viewer and `render`'s end-to-end test:

```sh
cargo run --bin rmdv -- examples/env/test.dme
```

`icons/test.dmi` is generated, not drawn: a 128x32 sheet of four 32x32 states (`floor`, `wall`,
`table`, `light`), each a flat colour with a 1px darker border and a magenta 6x6 notch in its top
left corner. The border makes a bleeding atlas rect obvious, and the notch makes a flipped `v`
obvious. It was written with `png::Encoder` plus an `add_ztxt_chunk("Description", ...)` holding the
usual `# BEGIN DMI ... # END DMI` block, which is the same shape `dmi`'s own tests build.
