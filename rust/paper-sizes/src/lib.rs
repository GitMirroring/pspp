//! # paper-sizes
//!
//! A library to detect the user's preferred paper size as well as
//! system-wide and per-user known sizes.  This is a Rust equivalent of
//! the library features in [libpaper].
//!
//! This crate does not provide the `paper` or `paperconf` programs.  Use
//! [libpaper] for those.
//!
//! [libpaper]: https://github.com/rrthomas/libpaper
//!
//! # License
//!
//! This crate is distributed under your choice of the following licenses:
//!
//! * The [MIT License].
//!
//! * The [GNU LGPL, version 2.1], or any later version.
//!
//! * The [Apache License, version 2.0].
//!
//! The `paperspecs` file in this crate is from [libpaper], which documents it
//! to be in the public domain.
//!
//! [MIT License]: https://opensource.org/license/mit
//! [GNU LGPL, version 2.1]: https://www.gnu.org/licenses/old-licenses/lgpl-2.1.en.html
//! [Apache License, version 2.0]: https://www.apache.org/licenses/LICENSE-2.0
#![warn(missing_docs)]
use std::{
    borrow::Cow,
    error::Error,
    fmt::Display,
    fs::File,
    io::{BufRead, BufReader, ErrorKind},
    ops::Not,
    path::{Path, PathBuf},
    str::FromStr,
};

use xdg::BaseDirectories;

#[cfg(target_os = "linux")]
mod locale;

include!(concat!(env!("OUT_DIR"), "/paperspecs.rs"));

static PAPERSIZE_FILENAME: &str = "papersize";
static PAPERSPECS_FILENAME: &str = "paperspecs";

enum DefaultPaper {
    Name(String),
    Size(PaperSize),
}

/// A unit of measurement used for [PaperSize]s.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unit {
    /// PostScript points (1/72 of an inch).
    Point,

    /// Inches.
    Inch,

    /// Millimeters.
    Millimeter,
}

/// [Unit] name cannot be parsed.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ParseUnitError;

impl FromStr for Unit {
    type Err = ParseUnitError;

    /// Parses the name of a unit in the form used in paperspecs files, one of
    /// `pt`, `in`, or `mm`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "pt" => Ok(Self::Point),
            "in" => Ok(Self::Inch),
            "mm" => Ok(Self::Millimeter),
            _ => Err(ParseUnitError),
        }
    }
}

impl Unit {
    /// Returns the name of the unit in the form used in paperspecs files.
    pub fn name(&self) -> &'static str {
        match self {
            Unit::Point => "pt",
            Unit::Inch => "in",
            Unit::Millimeter => "mm",
        }
    }

    /// Returns the number of `other` in one unit of `self`.
    ///
    /// To convert a quantity of unit `a` into unit `b`, multiply by
    /// `a.as_unit(b)`.
    fn as_unit(&self, other: Unit) -> f64 {
        match (*self, other) {
            (Unit::Point, Unit::Point) => 1.0,
            (Unit::Point, Unit::Inch) => 1.0 / 72.0,
            (Unit::Point, Unit::Millimeter) => 25.4 / 72.0,
            (Unit::Inch, Unit::Point) => 72.0,
            (Unit::Inch, Unit::Inch) => 1.0,
            (Unit::Inch, Unit::Millimeter) => 25.4,
            (Unit::Millimeter, Unit::Point) => 72.0 / 25.4,
            (Unit::Millimeter, Unit::Inch) => 1.0 / 25.4,
            (Unit::Millimeter, Unit::Millimeter) => 1.0,
        }
    }
}

impl Display for Unit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// The size of a piece of paper.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PaperSize {
    /// The paper's width, in [unit](Self::unit).
    pub width: f64,

    /// The paper's height (or length), in [unit](Self::unit).
    pub height: f64,

    /// The unit of [width](Self::width) and [height](Self::height).
    pub unit: Unit,
}

impl Default for PaperSize {
    /// A4, the internationally standard paper size.
    fn default() -> Self {
        Self::new(210.0, 297.0, Unit::Millimeter)
    }
}

impl PaperSize {
    /// Constructs a new `PaperSize`.
    pub fn new(width: f64, height: f64, unit: Unit) -> Self {
        Self {
            width,
            height,
            unit,
        }
    }

    /// Returns this paper size converted to `unit`.
    pub fn as_unit(&self, unit: Unit) -> PaperSize {
        Self {
            width: self.width * self.unit.as_unit(unit),
            height: self.height * self.unit.as_unit(unit),
            unit,
        }
    }

    /// Returns this paper size's `width` and `height`, discarding the unit.
    pub fn into_width_height(self) -> (f64, f64) {
        (self.width, self.height)
    }

    /// Returns true if `self` and `other` are equal to the nearest `unit`,
    /// false otherwise.
    pub fn eq_rounded(&self, other: &Self, unit: Unit) -> bool {
        let (aw, ah) = self.as_unit(unit).into_width_height();
        let (bw, bh) = other.as_unit(unit).into_width_height();
        aw.round() == bw.round() && ah.round() == bh.round()
    }
}

/// An error parsing a [PaperSpec].
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ParsePaperSpecError {
    /// Invalid paper height.
    InvalidHeight,

    /// Invalid paper width.
    InvalidWidth,

    /// Invalid unit of measurement.
    InvalidUnit,

    /// Missing field in paper specification.
    MissingField,
}

impl Error for ParsePaperSpecError {}

impl Display for ParsePaperSpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ParsePaperSpecError::InvalidHeight => write!(f, "Invalid paper height."),
            ParsePaperSpecError::InvalidWidth => write!(f, "Invalid paper width."),
            ParsePaperSpecError::InvalidUnit => write!(f, "Invalid unit of measurement."),
            ParsePaperSpecError::MissingField => write!(f, "Missing field in paper specification."),
        }
    }
}

/// A named [PaperSize].
#[derive(Clone, Debug, PartialEq)]
pub struct PaperSpec {
    /// The paper's name, such as `A4` or `Letter`.
    pub name: Cow<'static, str>,

    /// The paper's size.
    pub size: PaperSize,
}

impl PaperSpec {
    /// Construct a new `PaperSpec`.
    pub fn new(name: impl Into<Cow<'static, str>>, size: PaperSize) -> Self {
        Self {
            name: name.into(),
            size,
        }
    }
}

impl FromStr for PaperSpec {
    type Err = ParsePaperSpecError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let mut tokens = s.split(',').map(|s| s.trim());
        if let Some(name) = tokens.next()
            && let Some(width) = tokens.next()
            && let Some(height) = tokens.next()
            && let Some(unit) = tokens.next()
        {
            let width = f64::from_str(width).map_err(|_| ParsePaperSpecError::InvalidWidth)?;
            let height = f64::from_str(height).map_err(|_| ParsePaperSpecError::InvalidHeight)?;
            let unit = Unit::from_str(unit).map_err(|_| ParsePaperSpecError::InvalidUnit)?;
            Ok(Self {
                name: String::from(name).into(),
                size: PaperSize::new(width, height, unit),
            })
        } else {
            Err(ParsePaperSpecError::MissingField)
        }
    }
}

/// An error encountered building a [Catalog].
#[derive(Debug)]
pub enum CatalogBuildError {
    /// Line {line_number}: {error}
    ParseError {
        /// The file where the parse error occurred.
        path: PathBuf,

        /// The 1-based line number on which the parse error occurred.
        line_number: usize,

        /// The parse error.
        error: ParsePaperSpecError,
    },

    /// I/O error.
    IoError {
        /// The file where the I/O error occurred.
        path: PathBuf,

        /// Error details.
        error: std::io::Error,
    },
}

impl Error for CatalogBuildError {}

impl Display for CatalogBuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CatalogBuildError::ParseError {
                path,
                line_number,
                error,
            } => write!(f, "{}:{line_number}: {error}", path.display()),
            CatalogBuildError::IoError { path, error } => {
                write!(f, "{}: {error}", path.display())
            }
        }
    }
}

/// A builder for constructing a [Catalog].
///
/// `CatalogBuilder` allows control over the process of constructing a
/// [Catalog].  If the default options are acceptable, [Catalog::new] bypasses
/// the need for `CatalogBuilder`.
pub struct CatalogBuilder<'a> {
    papersize: Option<Option<&'a str>>,
    use_locale: bool,
    user_config_dir: Option<Option<&'a Path>>,
    system_config_dir: Option<&'a Path>,
    error_cb: Box<dyn FnMut(CatalogBuildError) + 'a>,
}

impl<'a> Default for CatalogBuilder<'a> {
    fn default() -> Self {
        Self {
            use_locale: true,
            papersize: None,
            user_config_dir: None,
            system_config_dir: Some(Path::new("/etc")),
            error_cb: Box::new(drop),
        }
    }
}

fn fallback_specs() -> (Vec<PaperSpec>, PaperSpec) {
    let specs = STANDARD_PAPERSPECS.into_iter().cloned().collect::<Vec<_>>();
    let default = specs.first().unwrap().clone();
    (specs, default)
}

fn read_specs<E>(
    user_config_dir: Option<&Path>,
    system_config_dir: Option<&Path>,
    mut error_cb: E,
) -> Option<(Vec<PaperSpec>, PaperSpec)>
where
    E: FnMut(CatalogBuildError),
{
    fn read_paperspecs_file(
        directory: Option<&Path>,
        error_cb: &mut dyn FnMut(CatalogBuildError),
    ) -> Vec<PaperSpec> {
        let mut specs = Vec::new();
        if let Some(directory) = directory {
            let path = directory.join(PAPERSPECS_FILENAME);
            match File::open(&path) {
                Ok(file) => {
                    let reader = BufReader::new(file);
                    for (line, line_number) in reader.lines().zip(1..) {
                        match line
                            .map_err(|error| CatalogBuildError::IoError {
                                path: path.clone(),
                                error,
                            })
                            .and_then(|line| {
                                PaperSpec::from_str(&line).map_err(|error| {
                                    CatalogBuildError::ParseError {
                                        path: path.clone(),
                                        line_number,
                                        error,
                                    }
                                })
                            }) {
                            Ok(spec) => specs.push(spec),
                            Err(error) => error_cb(error),
                        }
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => (),
                Err(error) => error_cb(CatalogBuildError::IoError { path, error }),
            }
        }
        specs
    }

    let user_specs = read_paperspecs_file(user_config_dir, &mut error_cb);
    let system_specs = read_paperspecs_file(system_config_dir, &mut error_cb);
    let default_spec = system_specs.first().or(user_specs.first())?.clone();
    Some((
        user_specs.into_iter().chain(system_specs).collect(),
        default_spec,
    ))
}

fn default_paper<E>(
    papersize: Option<Option<&str>>,
    user_config_dir: Option<&Path>,
    _use_locale: bool,
    system_config_dir: Option<&Path>,
    default: &PaperSpec,
    mut error_cb: E,
) -> DefaultPaper
where
    E: FnMut(CatalogBuildError),
{
    fn read_papersize_file<P, E>(path: P, mut error_cb: E) -> Option<String>
    where
        P: AsRef<Path>,
        E: FnMut(CatalogBuildError),
    {
        fn inner(path: &Path) -> std::io::Result<Option<String>> {
            let file = BufReader::new(File::open(path)?);
            let line = file.lines().next().unwrap_or(Ok(String::new()))?;
            let name = line.split(',').next().unwrap_or("");
            Ok(name.is_empty().not().then(|| name.into()))
        }
        let path = path.as_ref();
        match inner(path) {
            Ok(result) => result,
            Err(error) => {
                if error.kind() != ErrorKind::NotFound {
                    error_cb(CatalogBuildError::IoError {
                        path: path.to_path_buf(),
                        error,
                    });
                }
                None
            }
        }
    }

    // Use `PAPERSIZE` from the environment (or from the override).
    let env_var;
    let paper_name = match papersize {
        Some(paper_name) => paper_name,
        None => {
            env_var = std::env::var("PAPERSIZE").ok();
            env_var.as_deref()
        }
    };
    if let Some(paper_name) = paper_name
        && !paper_name.is_empty()
    {
        return DefaultPaper::Name(paper_name.into());
    }

    // Then try the user configuration directory.
    if let Some(dir) = user_config_dir
        && let path = dir.join(PAPERSIZE_FILENAME)
        && let Some(paper_name) = read_papersize_file(path, &mut error_cb)
    {
        return DefaultPaper::Name(paper_name);
    }

    // Then try the locale.
    #[cfg(target_os = "linux")]
    if _use_locale && let Some(paper_size) = locale::locale_paper_size() {
        return DefaultPaper::Size(paper_size);
    }

    if let Some(system_config_dir) = system_config_dir
        && let Some(paper_name) =
            read_papersize_file(system_config_dir.join(PAPERSIZE_FILENAME), &mut error_cb)
    {
        return DefaultPaper::Name(paper_name);
    }

    // Otherwise take it from the default papers.
    DefaultPaper::Name(default.name.as_ref().into())
}

impl<'a> CatalogBuilder<'a> {
    /// Constructs a new `CatalogBuilder` with default settings.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a [Catalog] and chooses a default paper size by reading the by
    /// reading `paperspecs` and `papersize` files and examining the
    /// environment and (on GNU/Linux) locale.
    ///
    /// If no system or user `paperspecs` files exist, or if they exist but they
    /// contain no valid paper specifications, then this method uses the
    /// standard paper sizes in [`STANDARD_PAPERSPECS`].  This is usually a
    /// reasonable fallback.
    pub fn build(self) -> Catalog {
        self.build_inner(|user_config_dir, system_config_dir, error_cb| {
            Some(
                read_specs(user_config_dir, system_config_dir, error_cb)
                    .unwrap_or_else(fallback_specs),
            )
        })
        .unwrap()
    }

    /// Builds a [Catalog] from [`STANDARD_PAPERSPECS`] and chooses a default
    /// paper size by reading the by reading `papersize` files and examining the
    /// environment and (on GNU/Linux) locale.
    ///
    /// This is a reasonable choice if it is unlikely for `paperspecs` to be
    /// installed but it is still desirable to detect a default paper size.
    pub fn build_from_fallback(self) -> Catalog {
        self.build_inner(|_, _, _| Some(fallback_specs())).unwrap()
    }

    /// Tries to build a [Catalog] and chooses a default paper size by reading
    /// the by reading `paperspecs` and `papersize` files and examining the
    /// environment and (on GNU/Linux) locale.
    ///
    /// If no system or user `paperspecs` files exist, or if they exist but they
    /// contain no valid paper specifications, this method fails and returns
    /// `None`.
    pub fn build_without_fallback(self) -> Option<Catalog> {
        self.build_inner(|user_config_dir, system_config_dir, error_cb| {
            read_specs(user_config_dir, system_config_dir, error_cb)
        })
    }

    /// Sets `papersize` to be used for the value of the `PAPERSIZE` environment
    /// variable, instead of obtaining it from the process environment.  `None`
    /// means that the environment variable is assumed to be empty or absent.
    pub fn with_papersize_value(self, papersize: Option<&'a str>) -> Self {
        Self {
            papersize: Some(papersize),
            ..self
        }
    }

    /// On GNU/Linux, by default, `CatalogBuilder` will consider the paper size
    /// setting in the glibc locale `LC_PAPER`.  This method disables this
    /// feature.
    ///
    /// This setting has no effect on other operating systems, which do not
    /// support paper size as part of their locales.
    pub fn without_locale(self) -> Self {
        Self {
            use_locale: false,
            ..self
        }
    }

    /// Overrides the name of the user-specific configuration directory.
    ///
    /// This directory is searched for the user-specified `paperspecs` and
    /// `papersize` files.  It defaults to `$XDG_CONFIG_HOME`, which is usually
    /// `$HOME/.config`.
    ///
    /// Passing `None` will disable reading `paperspec` or `papersize` from the
    /// user configuration directory.
    pub fn with_user_config_dir(self, user_config_dir: Option<&'a Path>) -> Self {
        Self {
            user_config_dir: Some(user_config_dir),
            ..self
        }
    }

    /// Overrides the name of the system configuration directory.
    ///
    /// This directory is searched for the system `paperspecs` and `papersize`
    /// files.  It defaults to `/etc`.
    ///
    /// Passing `None` will disable reading `paperspec` or `papersize` from the
    /// system configuration directory.
    pub fn with_system_config_dir(self, system_config_dir: Option<&'a Path>) -> Self {
        Self {
            system_config_dir,
            ..self
        }
    }

    /// Sets an error reporting callback.
    ///
    /// By default, [CatalogBuilder] ignores errors while building the catalog.
    /// The `error_cb` callback allows the caller to receive information about
    /// these errors.
    ///
    /// It is not considered an error if `paperspecs` or `papersize` files do
    /// not exist.
    pub fn with_error_callback(self, error_cb: Box<dyn FnMut(CatalogBuildError) + 'a>) -> Self {
        Self { error_cb, ..self }
    }

    fn build_inner<F>(mut self, f: F) -> Option<Catalog>
    where
        F: Fn(
            Option<&Path>,
            Option<&Path>,
            &mut Box<dyn FnMut(CatalogBuildError) + 'a>,
        ) -> Option<(Vec<PaperSpec>, PaperSpec)>,
    {
        let base_directories;
        let user_config_dir = match self.user_config_dir {
            Some(user_config_dir) => user_config_dir,
            None => {
                base_directories = BaseDirectories::new();
                base_directories.config_home.as_deref()
            }
        };
        let (specs, default) = f(user_config_dir, self.system_config_dir, &mut self.error_cb)?;
        let default = match default_paper(
            self.papersize,
            user_config_dir,
            self.use_locale,
            self.system_config_dir,
            &default,
            &mut self.error_cb,
        ) {
            DefaultPaper::Name(name) => specs
                .iter()
                .find(|spec| spec.name.eq_ignore_ascii_case(&name))
                .cloned()
                .unwrap_or(default),
            DefaultPaper::Size(size) => specs
                .iter()
                .find(|spec| spec.size.eq_rounded(&size, Unit::Point))
                .cloned()
                .unwrap_or_else(|| PaperSpec::new(Cow::from("Locale"), size)),
        };

        Some(Catalog { specs, default })
    }
}

/// A collection of [PaperSpec]s and a default paper size.
pub struct Catalog {
    specs: Vec<PaperSpec>,
    default: PaperSpec,
}

impl Default for Catalog {
    fn default() -> Self {
        Self::builder().build()
    }
}

impl Catalog {
    /// Constructs a new [CatalogBuilder].
    pub fn builder<'a>() -> CatalogBuilder<'a> {
        CatalogBuilder::new()
    }

    /// Constructs a new catalog by reading `paperspecs` and `papersize` files
    /// and examining the environment.
    ///
    /// This is equivalent to `Catalog::builder().build()`.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the contents of the catalog, as a nonempty list of user-specific
    /// paper sizes, followed by system paper sizes.
    pub fn specs(&self) -> &[PaperSpec] {
        &self.specs
    }

    /// Returns the default paper size.
    ///
    /// This paper size might not be in the catalog's list of [PaperSpec]s
    /// because the default can be specified in terms of measurements rather
    /// than as a name.
    pub fn default_paper(&self) -> &PaperSpec {
        &self.default
    }

    /// Returns the first [PaperSpec] in the catalog with the given `size` (to
    /// the nearest PostScript point).
    pub fn get_by_size(&self, size: &PaperSize) -> Option<&PaperSpec> {
        self.specs
            .iter()
            .find(|spec| spec.size.eq_rounded(size, Unit::Point))
    }

    /// Returns the first [PaperSpec] in the catalog whose name equals `name`,
    /// disregarding ASCII case.
    pub fn get_by_name(&self, name: &str) -> Option<&PaperSpec> {
        self.specs
            .iter()
            .find(|spec| spec.name.eq_ignore_ascii_case(name))
    }
}

#[cfg(test)]
mod tests {
    use std::{borrow::Cow, path::Path};

    use crate::{
        CatalogBuildError, CatalogBuilder, PaperSize, PaperSpec, ParsePaperSpecError, Unit, locale,
    };

    #[test]
    fn unit() {
        assert_eq!(Unit::Point.to_string(), "pt");
        assert_eq!(Unit::Millimeter.to_string(), "mm");
        assert_eq!(Unit::Inch.to_string(), "in");

        assert_eq!("pt".parse(), Ok(Unit::Point));
        assert_eq!("mm".parse(), Ok(Unit::Millimeter));
        assert_eq!("in".parse(), Ok(Unit::Inch));

        assert_eq!(
            format!("{:.3}", 1.0 * Unit::Inch.as_unit(Unit::Millimeter)),
            "25.400"
        );
        assert_eq!(
            format!("{:.3}", 1.0 * Unit::Inch.as_unit(Unit::Inch)),
            "1.000"
        );
        assert_eq!(
            format!("{:.3}", 1.0 * Unit::Inch.as_unit(Unit::Point)),
            "72.000"
        );
        assert_eq!(
            format!("{:.3}", 36.0 * Unit::Point.as_unit(Unit::Millimeter)),
            "12.700"
        );
        assert_eq!(
            format!("{:.3}", 36.0 * Unit::Point.as_unit(Unit::Inch)),
            "0.500"
        );
        assert_eq!(
            format!("{:.3}", 36.0 * Unit::Point.as_unit(Unit::Point)),
            "36.000"
        );
        assert_eq!(
            format!("{:.3}", 12.7 * Unit::Millimeter.as_unit(Unit::Millimeter)),
            "12.700"
        );
        assert_eq!(
            format!("{:.3}", 12.7 * Unit::Millimeter.as_unit(Unit::Inch)),
            "0.500"
        );
        assert_eq!(
            format!("{:.3}", 12.7 * Unit::Millimeter.as_unit(Unit::Point)),
            "36.000"
        );
    }

    #[test]
    fn paperspec() {
        assert_eq!(
            "Letter,8.5,11,in".parse(),
            Ok(PaperSpec::new(
                Cow::from("Letter"),
                PaperSize::new(8.5, 11.0, Unit::Inch)
            ))
        );
    }

    #[test]
    fn default() {
        // Default from $PAPERSIZE.
        assert_eq!(
            CatalogBuilder::new()
                .with_papersize_value(Some("legal"))
                .with_user_config_dir(Some(Path::new("testdata/td1")))
                .without_locale()
                .build_from_fallback()
                .default_paper(),
            &PaperSpec::new(Cow::from("Legal"), PaperSize::new(8.5, 14.0, Unit::Inch))
        );

        // Default from user_config_dir.
        assert_eq!(
            CatalogBuilder::new()
                .with_papersize_value(None)
                .with_user_config_dir(Some(Path::new("testdata/td1")))
                .without_locale()
                .build_from_fallback()
                .default_paper(),
            &PaperSpec::new(Cow::from("Ledger"), PaperSize::new(17.0, 11.0, Unit::Inch))
        );

        // Default from system_config_dir.
        assert_eq!(
            CatalogBuilder::new()
                .with_papersize_value(None)
                .with_user_config_dir(None)
                .with_system_config_dir(Some(Path::new("testdata/td2")))
                .without_locale()
                .build_from_fallback()
                .default_paper(),
            &PaperSpec::new(
                Cow::from("Executive"),
                PaperSize::new(7.25, 10.5, Unit::Inch)
            )
        );

        // Default from the first system paper size.
        assert_eq!(
            CatalogBuilder::new()
                .with_papersize_value(None)
                .with_user_config_dir(None)
                .with_system_config_dir(Some(Path::new("testdata/td2")))
                .without_locale()
                .build()
                .default_paper(),
            &PaperSpec::new(
                Cow::from("A0"),
                PaperSize::new(841.0, 1189.0, Unit::Millimeter)
            )
        );

        // Default from the first user paper size.
        assert_eq!(
            CatalogBuilder::new()
                .with_papersize_value(None)
                .with_user_config_dir(Some(Path::new("testdata/td3")))
                .with_system_config_dir(None)
                .without_locale()
                .build()
                .default_paper(),
            &PaperSpec::new(
                Cow::from("B0"),
                PaperSize::new(1000.0, 1414.0, Unit::Millimeter)
            )
        );

        // Default when nothing can be read and fallback triggers.
        assert_eq!(
            CatalogBuilder::new()
                .with_papersize_value(None)
                .with_user_config_dir(None)
                .with_system_config_dir(None)
                .without_locale()
                .build()
                .default_paper(),
            &PaperSpec::new(
                Cow::from("A4"),
                PaperSize::new(210.0, 297.0, Unit::Millimeter)
            )
        );

        // Verify that nothing can be read in the previous case.
        assert!(
            CatalogBuilder::new()
                .with_papersize_value(None)
                .with_user_config_dir(None)
                .with_system_config_dir(None)
                .without_locale()
                .build_without_fallback()
                .is_none()
        );
    }

    #[test]
    fn errors() {
        // Missing files are not errors.
        let mut errors = Vec::new();
        let _ = CatalogBuilder::new()
            .with_papersize_value(None)
            .with_user_config_dir(Some(Path::new("nonexistent/user")))
            .with_system_config_dir(Some(Path::new("nonexistent/system")))
            .without_locale()
            .with_error_callback(Box::new(|error| errors.push(error)))
            .build()
            .default_paper();
        assert_eq!(errors.len(), 0);

        // Test parse errors.
        let mut errors = Vec::new();
        let _ = CatalogBuilder::new()
            .with_papersize_value(None)
            .with_user_config_dir(None)
            .with_system_config_dir(Some(Path::new("testdata/td4")))
            .without_locale()
            .with_error_callback(Box::new(|error| errors.push(error)))
            .build()
            .default_paper();

        assert_eq!(errors.len(), 4);
        for ((error, expect_line_number), expect_error) in errors.iter().zip(1..).zip([
            ParsePaperSpecError::MissingField,
            ParsePaperSpecError::InvalidWidth,
            ParsePaperSpecError::InvalidHeight,
            ParsePaperSpecError::InvalidUnit,
        ]) {
            let CatalogBuildError::ParseError {
                path,
                line_number,
                error,
            } = error
            else {
                unreachable!()
            };
            assert_eq!(path.as_path(), Path::new("testdata/td4/paperspecs"));
            assert_eq!(*line_number, expect_line_number);
            assert_eq!(*error, expect_error);
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn lc_paper() {
        // Haven't figured out a good way to test this.
        //
        // I expect that all locales default to either A4 or letter-sized paper,
        // so just check for that.
        if let Some(size) = locale::locale_paper_size() {
            assert_eq!(size.unit, Unit::Millimeter);
            let (w, h) = size.into_width_height();
            assert!(
                (w, h) == (210.0, 297.0) || (w, h) == (216.0, 279.0),
                "Expected A4 (210x297) or letter (216x279) paper, got {w}x{h} mm"
            );
        }
    }
}
