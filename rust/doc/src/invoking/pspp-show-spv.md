# Inspecting SPSS Viewer Files

The `pspp show-spv` command reads SPSS Viewer (SPV) files, whose names
usually have an `.spv` extension, and produces a report.  The basic
syntax is:

```
pspp show-spv <MODE> <INPUT> [OUTPUT]
```

where `<MODE>` is a mode of operation (see below) and `<INPUT>` is the
SPV file to read, and `[OUTPUT]` is the output file name.  If
`[OUTPUT]` is omitted, output is written to the terminal.

The following `<MODE>`s are accepted:

* `dir`: Outputs a table of contents for the SPV file, listing every
  selected object, which by default is every object except for hidden
  ones.

  The following additional option for `dir` is intended mainly for use
  by PSPP developers:

  - `--member-names`: Also show the names of the ZIP file members
    associated with each object.

* `get-table-look`: Extracts the TableLook from the first table in the
  selected objects and outputs it in TableLook XML format.  The output
  file should have an `.stt` extension.

  Use `-` for `<INPUT>` to instead write the default TableLook.

* `convert-table-look`: Reads an `.stt` or `.tlo` TableLook file as
  `<INPUT>` and outputs it in TableLook XML format.  The output file
  should have an `.stt` extension.

  This is useful for converting a TableLook `.tlo` file from SPSS 15
  or earlier into the newer `.stt` format.

## Options

These options apply to any `<MODE>` that reads an SPV file:

* `-p <PASSWORD>`  
  `--password=<PASSWORD>`  
  Specifies the password for reading an encrypted SPV file.

  PSPP reads, but does not write, encrypted SPV files.

  > ⚠️ The password (and other command-line options) may be visible to
  other users on multiuser systems.

## Input Selection Options

Commands that read an SPV file operate, by default, on all of the
objects in the file, except for objects that are not visible in the
output viewer window.  The user may specify these options to select a
subset of the input objects.  When multiple options are used, only
objects that satisfy all of them are selected:

* `--select=[^]CLASS...`  
  Include only objects of the given `CLASS`; with leading `^`, include
  only objects not in the class.  Use commas to separate multiple
  classes.  The supported classes are:

  * `charts`
  * `headings`
  * `logs`
  * `models`
  * `tables`
  * `texts`
  * `trees`
  * `warnings`
  * `outlineheaders`
  * `pagetitle`
  * `notes`
  * `unknown`
  * `other`

* `--commands=[^]COMMAND...`  
  `--subtypes=[^]SUBTYPE...`  
  `--labels=[^]LABEL...`  
  Include only objects with the specified `COMMAND`, `SUBTYPE`, or
  `LABEL`.  With a leading `^`, include only the objects that do not
  match.  Multiple values may be specified separated by commas.  An
  asterisk at the end of a value acts as a wildcard.

  The `--command` option matches command identifiers, case
  insensitively.  All of the objects produced by a single command use
  the same, unique command identifier.  Command identifiers are
  always in English regardless of the language used for output.  They
  often differ from the command name in PSPP syntax.  Use the
  `pspp-output` program's `dir` command to print command identifiers
  in particular output.

  The `--subtypes` option matches particular tables within a command,
  case insensitively.  Subtypes are not necessarily unique: two
  commands that produce similar output tables may use the same
  subtype.  Only tables have subtypes, so specifying `--subtypes` will
  exclude other kinds of objects.  Subtypes are always in English and
  `dir` will print them.

  The `--labels` option matches the labels in table output (that is,
  the table titles).  Labels are affected by the output language,
  variable names and labels, split file settings, and other factors.

* `--nth-commands=N...`  
  Include only objects from the `N`th command that matches `--command`
  (or the `N`th command overall if `--command` is not specified),
  where `N` is 1 for the first command, 2 for the second, and so on.

* `--instances=INSTANCE...`  
  Include the specified `INSTANCE` of an object that matches the other
  criteria within a single command.  `INSTANCE` may be a number (1 for
  the first instance, 2 for the second, and so on) or `last` for the
  last instance.

* `--show-hidden`  
  Include hidden output objects in the output.  By default, they are
  excluded.

* `--or`  
  Separates two sets of selection options.  Objects selected by
  either set of options are included in the output.

The following additional input selection options are intended mainly
for use by PSPP developers:

* `--errors`  
  Include only objects that cause an error when read.  With the
  `convert` command, this is most useful in conjunction with the
  `--force` option.

*  `--members=MEMBER...`  
  Include only the objects that include a listed Zip file `MEMBER`.
  More than one name may be included, comma-separated.  The members in
  an SPV file may be listed with the `dir` command by adding the
  `--member-names` option or with `zipinfo` or another program to view
  Zip files.  Error messages that `pspp-output` prints when it reads
  SPV files also often include member names.
