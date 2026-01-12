# The `tectonic_xetex_format` crate

[![](http://meritbadge.herokuapp.com/jxoesneon-tectonic-xetex_format)](https://crates.io/crates/jxoesneon-tectonic-xetex_format)

> [!NOTE]
> This crate is part of the **FerroTeX** project, a specialized fork of Tectonic.
> It is published to crates.io as `jxoesneon-tectonic-xetex_format`.

This crate is part of [the Tectonic
project](https://tectonic-typesetting.github.io/en-US/). It provides
introspection of the internal data structures of the Tectonic/[XeTeX] engine and
their serialization into "format files".

[XeTeX]: http://xetex.sourceforge.net/

- [API documentation](https://docs.rs/tectonic_xetex_format/).
- [Main Git repository](https://github.com/jxoesneon/tectonic/).

This crate has two main uses: you can use it to decode an existing format file
and introspect the detailed setup that it encodes; or you can use it to emit a C
header file defining magic constants in the engine implementation. The former
usage isn't fully developed yet, but many of the key pieces have been
implemented.


## Cargo features

This crate currently provides no [Cargo features][features].

[features]: https://doc.rust-lang.org/cargo/reference/features.html
