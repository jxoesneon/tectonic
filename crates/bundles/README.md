# The `tectonic_bundles` crate

[![](http://meritbadge.herokuapp.com/jxoesneon-tectonic-bundles)](https://crates.io/crates/jxoesneon-tectonic-bundles)

> [!NOTE]
> This crate is part of the **FerroTeX** project, a specialized fork of Tectonic.
> It is published to crates.io as `jxoesneon-tectonic-bundles`.

This crate is part of [the Tectonic
project](https://tectonic-typesetting.github.io/en-US/). It implements various
Tectonic "bundles" that provide access to collections of TeX support files.

- [API documentation](https://docs.rs/tectonic_bundles/).
- [Main Git repository](https://github.com/jxoesneon/tectonic/).


## Cargo features

This crate provides the following [Cargo features][features]:

[features]: https://doc.rust-lang.org/cargo/reference/features.html

- `geturl-curl`: use the [curl] crate to implement HTTP requests. In order for
  this to take effect, you must use `--no-default-features` because
  `geturl-reqwest` is a default feature and it takes precedence
- `geturl-reqwest`: use the [reqwest] crate to implement HTTP requests (enabled
  by default)
- `native-tls-vendored`: if using [reqwest], activate the `vendored` option in
  the [native-tls] crate, causing OpenSSL to be vendored

[curl]: https://docs.rs/curl/
[reqwest]: https://docs.rs/reqwest/
[native-tls]: https://github.com/sfackler/rust-native-tls
