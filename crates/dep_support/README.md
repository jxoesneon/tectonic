# The `tectonic_dep_support` crate

[![](http://meritbadge.herokuapp.com/jxoesneon-tectonic-dep_support)](https://crates.io/crates/jxoesneon-tectonic-dep_support)

> [!NOTE]
> This crate is part of the **FerroTeX** project, a specialized fork of Tectonic.
> It is published to crates.io as `jxoesneon-tectonic-dep_support`.

This crate is part of [the Tectonic
project](https://tectonic-typesetting.github.io/en-US/). It provides build-time
utilities for finding external library dependencies, allowing either
[pkg-config] or [vcpkg] to be used as the dep-finding backend, and providing
whatever fiddly features are needed to enable the Tectonic build process.

- [API documentation](https://docs.rs/tectonic_dep_support/).
- [Main Git repository](https://github.com/jxoesneon/tectonic/).

[pkg-config]: https://www.freedesktop.org/wiki/Software/pkg-config/
[vcpkg]: https://vcpkg.readthedocs.io/
