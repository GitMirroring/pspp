# Identifying Files

The `pspp identify` command identifies file types.  The syntax is:

```
pspp identify <FILE>
```

which reads `<FILE>` and identifies it based on its contents.  The
command prints one of the following on the terminal:

* `sav`, for an SPSS system file, `sav (encrypted)` if the file is
  encrypted.

* `por`, for an SPSS portable file.

* `pc+`, for an SPSS/PC+ data file.

* `spv`, for an SPSS Viewer file, or `spv (encrypted)` if the file is
  encrypted.

* `sps`, for an SPSS syntax file, `sps (encrypted)` if the file is
  encrypted, or `sps (low confidence)` if the file could be a syntax
  file or another kind of text file.

* `unknown`, for other kinds of files.
