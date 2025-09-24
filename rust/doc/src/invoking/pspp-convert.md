# Converting Files with `pspp convert`

The `pspp convert` command reads a file and writes it to another,
usually in a different format.  The basic syntax is:

```
pspp convert <INPUT> [OUTPUT]
```

which reads `<INPUT>` and writes a copy of it to `[OUTPUT]`, or to the
terminal if `[OUTPUT]` is omitted.

`pspp convert` can convert following kinds of files:

* Data files:

  When `<INPUT>` is an SPSS system file or portable file or an SPSS/PC+
  system file, then `[OUTPUT]` can be a data file format.  The
  currently supported data file formats are:

  - [Comma-separated value (CSV) files]
  - [SPSS system files]
  - [SPSS portable files]

  If `[OUTPUT]` is omitted, the default is to write the data to stdout
  in CSV format.

* Viewer files:

  When `<INPUT>` is an SPSS viewer (SPV) file, then `[OUTPUT]` may be
  any [PSPP output format], including:

  - [plain text]
  - [PDF]
  - [HTML]

  If `[OUTPUT]` is omitted, the default is to write the viewer file to
  stdout in plain text format.

[Converting SPSS Viewer Files]: pspp-convert-spv.md
[Comma-separated value (CSV) files]: output.md#comma-separated-value-output-csv
[SPSS system files]: output.md#system-file-output-sav
[SPSS portable files]: output.md#portable-file-output-por
[PSPP output format]: output.md
[plain text]: output.md#text-output-txt-and-text
[PDF]: output.md#pdf-output-pdf
[HTML]: output.md#html-output-htm-and-html

## Options

`pspp convert` accepts the following options:

* `-e <ENCODING>`  
  `--encoding=<ENCODING>`  
  Sets the character encoding used to read text strings in the input
  file.  This is not needed for new enough SPSS data files, but older
  data files do not identify their encoding, and PSPP cannot always
  guess correctly.

  `<ENCODING>` must be one of the labels for encodings in the
  [Encoding Standard].  PSPP does not support UTF-16 or EBCDIC
  encodings in data files.

  `pspp show encodings` can help figure out the correct encoding for a
  system file.

  [Encoding Standard]: https://encoding.spec.whatwg.org/#names-and-labels

* `--unicode`  
  For input from a system file, converts from the file's encoding to
  Unicode (UTF-8) encoding before writing the output.  If the input
  was not already in Unicode, then this causes string variables to be
  tripled in width.

* `-c <MAX_CASES>`  
  `--cases=<MAX_CASES>`  
  By default, all cases in the input are copied to the output.
  Specify this option to limit the number of copied cases.

* `-p <PASSWORD>`  
  `--password=<PASSWORD>`  
  Specifies the password for reading an encrypted SPSS system file.

  In addition to file encryption, SPSS supports a feature called
  "password encryption".  The password specified can be specified with
  or without "password encryption".

  `pspp convert` reads, but does not write, encrypted system files.

  > ⚠️ The password (and other command-line options) may be visible to
  > other users on multiuser systems.

* `-o <OUTPUT_OPTIONS>`  
  Adds `<OUTPUT_OPTIONS>` to the output engine configuration.  See
  [Output Drivers](output.md) for information on how to configure
  output.

  If no output driver is specified, the default output format is
  chosen based on `[OUTPUT]`'s extension.  If `[OUTPUT]` is omitted,
  output is written to stdout, either in [CSV] format if `<INPUT>` is
  a data file, or in [plain text] format if `<INPUT>` is an SPV file.

[CSV]: output.md#comma-separated-value-output-csv
