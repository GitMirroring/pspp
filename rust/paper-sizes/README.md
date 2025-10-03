# paper-sizes

`paper-sizes` is a library to detect the user's preferred paper size
as well as system-wide and per-user known sizes.

This is a Rust equivalent of the library features in [libpaper].

This crate does not provide the `paper` or `paperconf` programs.  Use
[libpaper] for those.

[libpaper]: https://github.com/rrthomas/libpaper

## Use

To obtain the default paper size, create a `Catalog`, then obtain the
default paper size:

```rust
use paper_sizes::Catalog;

let catalog = Catalog::new();
let default_size = catalog.default_paper().size;
```

See the documentation for more details.

## License

This crate is distributed under your choice of the following licenses:

* The [MIT License].

* The [GNU LGPL, version 2.1], or any later version.

* The [Apache License, version 2.0].

The `paperspecs` file in this crate is from [libpaper], which documents it
to be in the public domain.

[MIT License]: https://opensource.org/license/mit
[GNU LGPL, version 2.1]: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.en.html
[Apache License, version 2.0]: https://www.apache.org/licenses/LICENSE-2.0
