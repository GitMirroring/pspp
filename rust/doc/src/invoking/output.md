# Output Drivers

PSPP has output drivers for several formats.  This section documents
the supported formats and how they can be configured:

<!-- toc -->

# Text Output (`.txt` and `.text`)

PSPP can produce plain text output, drawing boxes using ASCII or
Unicode line drawing characters.

Plain text output is encoded in UTF-8.

This driver has the following options:

* `width = <columns>`  
  Sets the maximum page width to the specified number of columns.  To
  fit in the given width, output table columns will be word-wrapped
  or, if necessary, tables will be broken into multiple chunks.  The
  default is no maximum width.

* `boxes = "unicode"`  
  `boxes = "ascii"`  
  Sets the style used for boxes in the output.  The following shows an
  example of each style:

  ```
    unicode       ascii
  ┌────┬────┐  +----+----+
  │    │    │  |    |    |
  ├────┼────┤  +----+----+
  │    │    │  |    |    |
  └────┴────┘  +----+----+
  ```

  Unicode boxes are generally more attractive but they can be harder
  to work with in some environments.  The default is `unicode`.

* `emphasis = "ansi"`  
  `emphasis = "overstrike"`  
  By default, text output is plain, without support for emphasis.  Set
  this to `ansi` to enable bold, italic, underline, and colors in
  terminals that support it using [ANSI escape codes], or to
  `overstrike` to enable bold and underline for emphasis with
  overstriking.

  [ANSI escape codes]: https://en.wikipedia.org/wiki/ANSI_escape_code

# PDF Output (`.pdf`)

This driver has the following options:

* <a name="page_setup">`page_setup = <PageSetup>`</a>  
  Sets the page size, margins, and other parameters.  The following
  sub-options are available:

  - `initial_page_number = <number>`  
    The page number to use for the first page of output.  The default
    is 1.

  - `paper = "<size>"`  
    Sets the page size.  `<size>` takes the form `<w>x<h><unit>`,
    e.g. `8.5x11in` or `210x297mm`, or the name of a standard paper
    size, such as `letter` or `a4`.  The default is system- and
    user-dependent.

  - `margins = "<trbl>"`  
    `margins = ["<tb>", "<lr>"]`  
    `margins = ["<t>", "<rl>", "<b>"]`  
    `margins = ["<t>", "<r>", "<b>", "<l>"]`  
    Sets the margins.  Each variable is a quoted string with a length
    and a unit, e.g. `10mm`.  The one-value form sets all margins to
    the same length; the two-value form sets the top and bottom
    margins separately from left and right; and so on.  The default is
    `0.5in`.

  - `orientation = "portrait"`  
    `orientation = "landscape"`  
    Controls the output page orientation.  The default is `"portrait"`.

  - `object_spacing = "<length>"`  
    Sets the vertical spacing between output objects, such as tables
    or text.  `<length>` includes a length and a unit, e.g. `10mm`.
    The default is `12pt`, or 1/6 of an inch.

  - `chart_spacing = "as_is"`  
    `chart_spacing = "full_height"`  
    `chart_spacing = "half_height"`  
    `chart_spacing = "quarter_height"`  
    Sets the size of charts and graphs in the output.  The default,
    `"as_is"`, uses the size specified in the charts themselves.  The
    other possibilities set chart size in terms of the height of the
    page.

  - `header = "<heading>"`  
    `footer = "<heading>"`  
    Sets the header, output at the top of each page, and the footer,
    output at the bottom of each page.  Both header and footer are
    expressed in HTML.  Only simple HTML features are supported,
    including the following HTML elements:

      * `<html>`, optionally, as a top-level element.

      * `<head>`, optionally, as a first element enclosing a `<style>`
        element with simple CSS.  Only the following CSS properties,
        for `p` elements only, are supported:

        - `color`: an RGB value, e.g. `#RRGGBB`.

        - `font-weight`: either `bold` or `normal`.

        - `font-style`: either `italic` or `normal`.

        - `text-decoration`: either `underline` or `normal`.

        - `font-family`: a font name.

        - `font-size`: a font size[^1].

      * `<p>`, optionally, for dividing text into multiple paragraphs.
        The `align` attribute is honored, as is the `text_align`
        property on the `style` attribute.

      * `<br>` for breaking a paragraph into lines.  Line breaks inside
        HTML are also treated as breaks.

      * `<b>`, `<i>`, `<u>`, `<strike>`, `<strong>`, and `<em>` for
        styling text.

      * `<font>`, with attributes `face`, `color`, and `size`[^1].

      * Other elements are accepted but ignored.

    Text in a header or footer may include any of the following
    variable references, which are expanded at output time:

      * `&[Date]`  
        `&[Time]`  
        The current date or time in the preferred format for the locale.

      * `&[Head1]`  
        `&[Head2]`  
        `&[Head3]`  
        `&[Head4]`  
        The name of the first-, second-, third-, or fourth-level
        heading corresponding to the first output item that appears on
        the page with the header or footer.

      * `&[PageTitle]`  
        The title set with the `TITLE` command.

      * `&[Filename]`  
        Name of the output file.

      * `&[Page]`  
        The page number.

    The following example shows how to put a centered title at the top
    of each page and a right-justified page number at the bottom:

    ```
    header = '<p align="center">&amp;[PageTitle]</p>'
    footer = '<p align="right">Page &amp;[Page]</p>'
    ```

[^1]: Font sizes on `<font>` and in CSS may be given as points,
    e.g. `10pt`, or as a bare number interpreted as follows:

    | Number |    Size |
    |-------:|--------:|
    |      1 |    6 pt |
    |      2 |  7.5 pt |
    |      3 |    9 pt |
    |      4 | 10.5 pt |
    |      5 | 13.5 pt |
    |      6 |   18 pt |
    |      7 |   27 pt |

# HTML Output (`.htm` and `.html`)

PSPP can produce HTML output.

HTML output is encoded in UTF-8.

This driver currently has no options.

# Comma-Separated Value Output (`.csv`)

PSPP can produce output in [comma-separated value] format.

CSV output is encoded in UTF-8.

This driver has the following general options:

* `quote = "<char>"`  
  A single character for quoting multi-line fields and fields that
  contain the delimiter.  The default is `"`.

* `delimiter = "<char>"`  
  A single character to separate fields.  The default is `,`.

The following additional options only affect output written by [`pspp
convert`](pspp-convert.md):

* `var_names = false`  
  By default, `pspp convert` writes the variable names as the first
  line of output.  With this option, `pspp convert` omits this line.

* `recode = true`  
  By default, `pspp convert` writes user-missing values to CSV output
  files as their regular values.  This options makes `pspp convert`
  write them the same way as system-missing values (as a single
  space).

* `labels = true`  
  By default, `pspp convert` writes variables' values to CSV output
  files.  With this option, `pspp convert` writes value labels.

* `print_formats = true`  
  By default, `pspp convert` writes numeric variables as plain
  numbers.  This option makes `pspp convert` honor variables' print
  formats.

* `decimal = "<char>"`  
  This option sets the character used as a decimal point in output.
  The default is `.`.

[comma-separated value]: https://en.wikipedia.org/wiki/Comma-separated_values

# JSON Output (`.json`)

PSPP can produce output in [JSON] format.

CSV output is encoded in UTF-8.

This driver currently has no options.

[JSON]: https://www.json.org/json-en.html

# SPSS Viewer Output (`.spv`)

PSPP can produce output in SPSS Viewer format.

This driver has the following options:

* `page_setup = <PageSetup>`  
  Sets the page size, margins, and other parameters.  `<PageSetup>`
  has the same form documented [for PDF format](#page_setup).

# System File Output (`.sav`)

PSPP can produce output in the form of SPSS system files, which
usually have a `.sav` extension.

This driver has the following options:

* `compression = "<compression>"`  
  Sets the kind of compression used for writing data in the system
  file.  `<compression>` must be one of the following:

  - `simple`: Uses a simple form of compression that saves space
    writing small integer values and string segments that are all
    spaces.  All versions of SPSS support simple compression.

  - `zlib`: Uses more advanced compression that saves space in more
    general cases.  Only SPSS 21 and later can read files written with
    `zlib` compression.

# Portable File Output (`.por`)

PSPP can produce output in the form of SPSS portable files, which
usually have a `.por` extension.

> The portable file format is mostly obsolete. The "system file"
> or .sav format should be used for writing new data files.

This driver has no options.
