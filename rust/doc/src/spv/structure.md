# Structure Member Format

A structure member lays out the high-level structure for a group of
output items such as heading, tables, and charts.  Structure members do
not include the details of tables and charts but instead refer to them
by their member names.

Structure members' XML files claim conformance with a collection of
XML Schemas.  These schemas are distributed, under a nonfree license,
with SPSS binaries.  Fortunately, the schemas are not necessary to
understand the structure members.  The schemas can even be deceptive
because they document elements and attributes that are not in the corpus
and do not document elements and attributes that are commonly found in
the corpus.

Structure members use a different XML namespace for each schema, but
these namespaces are not entirely consistent.  In some SPV files, for
example, the `viewer-tree` schema is associated with namespace
`http://xml.spss.com/spss/viewer-tree` and in others with
`http://xml.spss.com/spss/viewer/viewer-tree` (note the additional
`viewer/`).  Under either name, the schema URIs are not resolvable to
obtain the schemas themselves.

One may ignore all of the above in interpreting a structure member.
The actual XML has a simple and straightforward form that does not
require a reader to take schemas or namespaces into account.  A
structure member's root is `heading` element, which contains `heading`
or `container` elements (or a mix), forming a tree.  In turn,
`container` holds a `label` and one more child, usually `text` or
`table`.

<!-- toc -->

## Grammar

The following sections document the elements found in structure
members in a context-free grammar-like fashion.  Consider the following
example, which specifies the attributes and content for the `container`
element:

```
container
   :visibility=(visible | hidden)?
   :page-break-before=(always)?
   :text-align=(left | center)?
   :width=dimension
=> label (table | container_text | graph | model | object | image | tree)
```

Each attribute specification begins with `:` followed by the
attribute's name.  If the attribute's value has an easily specified
form, then `=` and its description follows the name.  Finally, if the
attribute is optional, the specification ends with `?`.  The following
value specifications are defined:

* `(A | B | ...)`  
  One of the listed literal strings.  If only one string is listed,
  it is the only acceptable value.  If `OTHER` is listed, then any
  string not explicitly listed is also accepted.

* `bool`  
  Either `true` or `false`.

* `dimension`  
  A floating-point number followed by a unit, e.g. `10pt`.  If the
  unit is omitted then points should be assumed.  The number and unit
  may be separated by white space.  The number might use `,` as the
  decimal point.

  The corpus includes the following units, which includes localized
  names for units.  A reader must understand these to properly
  interpret the dimensions:

  | Unit                     | Units per Inch | Names                                                                       |
  |:-------------------------|---------------:|:----------------------------------------------------------------------------|
  | Inch                     |              1 | `in`<BR>`인치`<BR>`pol.`<BR>`pulg.`<BR>`cala`<BR>`cali`<BR>`英吋`<BR>`英寸` |
  | Centimeter               |           2.54 | `cm`<BR>`см`                                                                |
  | Point                    |             72 | `pt`<BR>`пт`<BR>(empty string)                                              |
  | Device-independent pixel |             96 | `px`                                                                        |

* `real`  
  A floating-point number.

* `int`  
  An integer.

* `color`  
  A color in one of the forms `#RRGGBB` or `RRGGBB`, or the string
  `transparent`, or one of the standard Web color names.

* `ref`  
  `ref ELEMENT`  
  `ref(ELEM1 | ELEM2 | ...)`  
  The name from the `id` attribute in some element.  If one or more
  elements are named, the name must refer to one of those elements,
  otherwise any element is acceptable.

All elements have an optional `id` attribute.  If present, its value
must be unique.  In practice many elements are assigned `id` attributes
that are never referenced.

The content specification for an element supports the following
syntax:

* `ELEMENT`  
  An element.

* `A B`  
  A followed by B.

* `A | B | C`  
  One of A or B or C.

* `A?`  
  Zero or one instances of A.

* `A*`  
  Zero or more instances of A.

* `B+`  
  One or more instances of A.

* `(SUBEXPRESSION)`  
  Grouping for a subexpression.

* `EMPTY`  
  No content.

* `TEXT`  
  Text and CDATA.

Element and attribute names are sometimes suffixed by another name in
square brackets to distinguish different uses of the same name.  For
example, structure XML has two `text` elements, one inside `container`,
the other inside `pageParagraph`.  The former is defined as
`text[container_text]` and referenced as `container_text`, the latter
defined as `text[pageParagraph_text]` and referenced as
`pageParagraph_text`.

This language is used in the PSPP source code for parsing structure
and detail XML members.  Refer to `src/output/spv/structure-xml.grammar`
and `src/output/spv/detail-xml.grammar` for the full grammars.

The following example shows the contents of a typical structure
member for a DESCRIPTIVES procedure.  A real structure member is not
indented.  This example also omits most attributes, all XML namespace
information, and the CSS from the embedded HTML:

```
<?xml version="1.0" encoding="utf-8"?>
<heading>
  <label>Output</label>
  <heading commandName="Descriptives">
    <label>Descriptives</label>
    <container>
      <label>Title</label>
      <text commandName="Descriptives" type="title">
        <html lang="en">
<![CDATA[<head><style type="text/css">...</style></head><BR>Descriptives]]>
        </html>
      </text>
    </container>
    <container visibility="hidden">
      <label>Notes</label>
      <table commandName="Descriptives" subType="Notes" type="note">
        <tableStructure>
          <dataPath>00000000001_lightNotesData.bin</dataPath>
        </tableStructure>
      </table>
    </container>
    <container>
      <label>Descriptive Statistics</label>
      <table commandName="Descriptives" subType="Descriptive Statistics"
             type="table">
        <tableStructure>
          <dataPath>00000000002_lightTableData.bin</dataPath>
        </tableStructure>
      </table>
    </container>
  </heading>
</heading>
```

## The `heading` Element

```
heading[root_heading]
   :creator-version?
   :creator?
   :creation-date-time?
   :lockReader=bool?
   :schemaLocation?
=> label pageSetup? (container | heading)*

heading
   :creator-version?
   :commandName?
   :visibility[heading_visibility]=(collapsed)?
   :locale?
   :olang?
=> label (container | heading)*
```

A `heading` represents a tree of content that appears in an output
viewer window.  It contains a `label` text string that is shown in the
outline view ordinarily followed by content containers or further nested
(sub)-sections of output.  Unlike heading elements in HTML and other
common document formats, which precede the content that they head,
`heading` contains the elements that appear below the heading.

The root of a structure member is a special `heading`.  The direct
children of the root `heading` elements in all structure members in an
SPV file are siblings.  That is, the root `heading` in all of the
structure members conceptually represent the same node.  The root
heading's `label` is ignored (see [the `label`
element](#the-label-element)).  The root heading in the first
structure member in the Zip file may contain a `pageSetup` element.

The schema implies that any `heading` may contain a sequence of any
number of `heading` and `container` elements.  This does not work for
the root `heading` in practice, which must actually contain exactly one
`container` or `heading` child element.  Furthermore, if the root
heading's child is a `heading`, then the structure member's name must
end in `_heading.xml`; if it is a `container` child, then it must not.

The following attributes have been observed on both document root and
nested `heading` elements.

* `creator-version`  
  The version of the software that created this SPV file.  A string
  of the form `xxyyzzww` represents software version xx.yy.zz.ww,
  e.g. `21000001` is version 21.0.0.1.  Trailing pairs of zeros are
  sometimes omitted, so that `21`, `210000`, and `21000000` are all
  version 21.0.0.0 (and the corpus contains all three of those
  forms).

The following attributes have been observed on document root `heading`
elements only:

* `creator`  
  The directory in the file system of the software that created this
  SPV file.

* `creation-date-time`  
  The date and time at which the SPV file was written, in a
  locale-specific format, e.g. `Friday, May 16, 2014 6:47:37 PM PDT`
  or `lunedì 17 marzo 2014 3.15.48 CET` or even `Friday, December 5,
  2014 5:00:19 o'clock PM EST`.

* `lockReader`  
  Whether a reader should be allowed to edit the output.  The
  possible values are `true` and `false`.  The value `false` is by
  far the most common.

* `schemaLocation`  
  This is actually an XML Namespace attribute.  A reader may ignore
  it.

The following attributes have been observed only on nested `heading`
elements:

* `commandName`  
  A locale-invariant identifier for the command that produced the
  output, e.g. `Frequencies`, `T-Test`, `Non Par Corr`.

* `visibility`  
  If this attribute is absent, the heading's content is expanded in
  the outline view.  If it is set to `collapsed`, it is collapsed.
  (This attribute is never present in a root `heading` because the
  root node is always expanded when a file is loaded, even though the
  UI can be used to collapse it interactively.)

* `locale`  
  The locale used for output, in Windows format, which is similar to
  the format used in Unix with the underscore replaced by a hyphen,
  e.g. `en-US`, `en-GB`, `el-GR`, `sr-Cryl-RS`.

* `olang`  
  The output language, e.g. `en`, `it`, `es`, `de`, `pt-BR`.

## The `label` Element

```
label => TEXT
```

Every `heading` and `container` holds a `label` as its first child.
The label text is what appears in the outline pane of the GUI's viewer
window.  PSPP also puts it into the outline of PDF output.  The label
text doesn't appear in the output itself.

The text in `label` describes what it labels, often by naming the
statistical procedure that was executed, e.g. "Frequencies" or "T-Test".
Labels are often very generic, especially within a `container`, e.g.
"Title" or "Warnings" or "Notes".  Label text is localized according to
the output language, e.g. in Italian a frequency table procedure is
labeled "Frequenze".

The user can edit labels to be anything they want.  The corpus
contains a few examples of empty labels, ones that contain no text,
probably as a result of user editing.

The root `heading` in an SPV file has a `label`, like every
`heading`.  It normally contains "Output" but its content is disregarded
anyway.  The user cannot edit it.

## The `container` Element

```
container
   :visibility=(visible | hidden)?
   :page-break-before=(always | auto | avoid | left | right | inherit)?
   :text-align=(left | center)?
   :width=dimension
=> label (table | container_text | graph | model | object | image | tree)
```

A `container` serves to contain and label a `table`, `text`, or other
kind of item.

This element has the following attributes.

* `visibility`  
  Whether the container's content is displayed.  "Notes" tables are
  often hidden; other data is usually visible.  The default is
  `visible`.

* `page-break-before`  
  Whether to start the element at the beginning of a new page.  This
  attribute is usually not present.  The only value seen in the corpus
  is `always`.

* `text-align`  
  Alignment of text within the container.  Observed with nested
  `table` and `text` elements.

* `width`  
  The width of the container, e.g. `1097px`.

All of the elements that nest inside `container` (except the `label`)
have the following optional attribute.

* `commandName`  
  As on the `heading` element.  The corpus contains one example of
  where `commandName` is present but set to the empty string.

## The `text` Element (Inside `container`)

```
text[container_text]
  :type[text_type]=(title | log | text | page-title)
  :commandName?
  :creator-version?
=> html

html :lang=(en) => TEXT
```
This `text` element is nested inside a `container`.  There is a
[different `text` element that is nested inside a `pageParagraph`](#the-text-element-inside-pageparagraph).

This element has the following attributes.

* `commandName`  
  See [the `container` element](#the-container-element).  For output
  not specific to a command, this is simply `log`.

* `type`  
  The semantics of the text.

  Text with types `title`, `log`, and `text` appears directly in the
  output.  Text with type `page-title` sets the title that appears in
  page headers or footers when [`&[PageTitle]`](#pagetitle) is used.

* `creator-version`  
  As on the `heading` element.

### The `html` element

The `html` element inside `text` contains an HTML document as text
(or, in practice, as CDATA).  In some cases, the document starts with
`<html>` and ends with `</html>`, and in others the `html` element is
implied.  Generally the HTML includes a `head` element with a CSS
stylesheet.  The HTML body often begins with `<BR>`.  See [Embedded
HTML](#embedded-html) for details.

The `html` element has the following attributes:

* `lang`  
  This always contains `en` in the corpus.

> A few examples of typical text in the corpus:
>
> ```
> <html xmlns="http://www.w3.org/1999/xhtml" lang="en">&lt;head>&lt;style type="text/css">p{color:0;font-family:Monospaced;font-size:14pt;font-style:normal;font-weight:normal;text-decoration:none}&lt;/style>&lt;/head>&lt;BR>REGRESSION
>   /MISSING LISTWISE
>   /STATISTICS COEFF OUTS R ANOVA
>   /CRITERIA=PIN(.05) POUT(.10)
>   /NOORIGIN
>   /DEPENDENT Pvalues
>   /METHOD=ENTER MMN.</html>
> ```
>
> ```
> <html xmlns="http://www.w3.org/1999/xhtml" lang="en">&lt;head>&lt;style type="text/css">p{color:0;font-family:Monospaced;font-size:13pt;font-style:normal;font-weight:normal;text-decoration:none}&lt;/style>&lt;/head>&lt;BR>CROSSTABS&lt;BR>&amp;nbsp;&amp;nbsp;/TABLES=facrec&amp;nbsp;BY&amp;nbsp;nq1e&lt;BR>&amp;nbsp;&amp;nbsp;/FORMAT=AVALUE&amp;nbsp;TABLES&lt;BR>&amp;nbsp;&amp;nbsp;/CELLS=COUNT&amp;nbsp;ROW&lt;BR>&amp;nbsp;&amp;nbsp;/COUNT&amp;nbsp;ROUND&amp;nbsp;CELL.</html>
> ```
>
> ```
> <html xmlns="http://www.w3.org/1999/xhtml" lang="en">&lt;html>
>   &lt;head>
>     &lt;style type="text/css">
>       &lt;!--
>         p { font-style: normal; text-decoration: none; font-weight: bold; color: 000000; font-size: 14pt; font-family: Trebuchet MS }
>       -->
>     &lt;/style>
>
>   &lt;/head>
>   &lt;body>
>     &lt;b>&lt;font size="5" face="Times New Roman">                                                                     &lt;u>H&lt;/u>&lt;/font>&lt;u>&lt;font size="5" color="#000000" face="Times New Roman">ousehold
>     Income (In Thousands)&lt;/font>&lt;/u>&lt;font size="5" color="#000000" face="Times New Roman">
>     &lt;/font>&lt;/b>
>   &lt;/body>
> &lt;/html>
> </html>
> ```

## The `table` Element

```
table
   :VDPId?
   :ViZmlSource?
   :activePageId=int?
   :commandName
   :creator-version?
   :displayFiltering=bool?
   :maxNumCells=int?
   :orphanTolerance=int?
   :rowBreakNumber=int?
   :subType
   :tableId?
   :tableLookId?
   :type[table_type]=(table | note | warning)
=> tableProperties? tableStructure

tableStructure => path? dataPath csvPath?
```

This element has the following attributes.

* `commandName`  
  See [the `container` element](#the-container-element).

* `type`  
  One of `table`, `note`, or `warning`.

* `subType`  
  The locale-invariant command ID for the particular kind of output
  that this table represents in the procedure.  This can be the same
  as `commandName` e.g. `Frequencies`, or different, e.g. `Case
  Processing Summary`.  Generic subtypes `Notes` and `Warnings` are
  often used.

* `tableId`  
  A number that uniquely identifies the table within the SPV file,
  typically a large negative number such as `-4147135649387905023`.
  It is usually present.  For light binary members, this is the same
  as `table-id` in the [light detail member
  header](light-detail.md#header).

* `creator-version`  
  As on the `heading` element.  In the corpus, this is only present
  for version 21 and up and always includes all 8 digits.

This element contains the following:

* `tableProperties`  
  See [Legacy Properties](legacy-detail-xml.md#legacy-properties), for
  details.

* `tableStructure`  
  This element in turn contains:

  - Both `path` and `dataPath` for legacy members.

  - `dataPath` but not `path` for light detail binary members.

  - The usage of `csvPath` is rare and not yet understood.

  See [SPSS Viewer File Format](index.md) for more information on how
  structure members refer to tables.

## The `graph` Element

```
graph
   :VDPId?
   :ViZmlSource?
   :commandName?
   :creator-version?
   :dataMapId?
   :dataMapURI?
   :editor?
   :refMapId?
   :refMapURI?
   :csvFileIds?
   :csvFileNames?
=> dataPath? path csvPath?
```

This element represents a graph.  The `dataPath` and `path` elements
name the Zip members that give the details of the graph.  Normally, both
elements are present; there is only one counterexample in the corpus.

`csvPath` only appears in one SPV file in the corpus, for two graphs.
In these two cases, `dataPath`, `path`, and `csvPath` all appear.  These
`csvPath` name Zip members with names of the form `NUMBER_csv.bin`,
where `NUMBER` is a many-digit number and the same as the `csvFileIds`.
The named Zip members are CSV text files (despite the `.bin` extension).
The CSV files are encoded in UTF-8 and begin with a U+FEFF byte-order
marker.

## The `model` Element

```
model
   :PMMLContainerId?
   :PMMLId
   :StatXMLContainerId
   :VDPId
   :auxiliaryViewName
   :commandName
   :creator-version
   :mainViewName
=> ViZml? dataPath? path | pmmlContainerPath statsContainerPath

pmmlContainerPath => TEXT

statsContainerPath => TEXT

ViZml :viewName? => TEXT
```

This element represents a model.  The `dataPath` and `path` elements
name the Zip members that give the details of the model.  Normally, both
elements are present; there is only one counterexample in the corpus.

The details are unexplored.  The `ViZml` element contains base-64
encoded text, that decodes to a binary format with some embedded text
strings, and `path` names an Zip member that contains XML.
Alternatively, `pmmlContainerPath` and `statsContainerPath` name Zip
members with `.scf` extension.

## The `object` and `image` Elements

```
object
   :commandName?
   :type[object_type]=(unknown)?
   :uri
=> EMPTY

image
   :commandName?
   :VDPId
=> dataPath
```

These two elements represent an image in PNG format.  They are
equivalent and the corpus contains examples of both.  The only
difference is the syntax: for `object`, the `uri` attribute names the
Zip member that contains a PNG file; for `image`, the text of the inner
`dataPath` element names the Zip member.

PSPP writes `object` in output but there is no strong reason to
choose this form.

The corpus only contains PNG image files.

## The `tree` Element

```
tree
   :commandName
   :creator-version
   :name
   :type
=> dataPath path
```

This element represents a tree.  The `dataPath` and `path` elements
name the Zip members that give the details of the tree.  The details are
unexplored.

## Path Elements

```
dataPath => TEXT

path => TEXT

csvPath => TEXT
```

These element contain the name of the Zip members that hold details
for a container.  For tables:

- When a "light" format is used, only `dataPath` is present, and it
  names a `.bin` member of the Zip file that has `light` in its name,
  e.g. `0000000001437_lightTableData.bin`.  See [Light Detail Member
  Format](light-detail.md) for light format details.

- When the legacy format is used, both are present.  In this case,
  `dataPath` names a Zip member with a legacy binary format that
  contains relevant data (see [Legacy Detail Member Binary
  Format](legacy-detail-binary.md)), and `path` names a Zip member
  that uses an XML format (see [Legacy Detail Member XML Member
  Format](legacy-detail-xml.md)).

Graphs normally follow the legacy approach described above.  The
corpus contains one example of a graph with `path` but not `dataPath`.
The reason is unexplored.

Models use `path` but not `dataPath`.  See [`graph`
element](#the-graph-element), for more information.

These elements have no attributes.

## The `pageSetup` Element

```
pageSetup
   :initial-page-number=int?
   :chart-size=(as-is | full-height | half-height | quarter-height | OTHER)?
   :margin-left=dimension?
   :margin-right=dimension?
   :margin-top=dimension?
   :margin-bottom=dimension?
   :paper-height=dimension?
   :paper-width=dimension?
   :reference-orientation?
   :space-after=dimension?
=> pageHeader pageFooter

pageHeader => pageParagraph?

pageFooter => pageParagraph?

pageParagraph => pageParagraph_text
```

The `pageSetup` element has the following attributes.

* `initial-page-number`  
  The page number to put on the first page of printed output.
  Usually `1`.

* `chart-size`  
  One of the listed chart sizes, or some localization of `as-is`.

  > No localizations of other sizes have been observed, so PSPP
  > interprets any unknown value like `as-is`.

* `margin-left`  
  `margin-right`  
  `margin-top`  
  `margin-bottom`  
  Margin sizes, e.g. `0.25in`.

* `paper-height`  
  `paper-width`  
  Paper sizes.

* `reference-orientation`  
  Indicates the orientation of the output page.  This is most commonly
  `0deg` (portrait) or `90deg` (landscape).  Various localized
  versions also exist, such as `0grau` and `90 度`.

  > PSPP just looks at whether the value starts with `0` or `90`
  > because all of the localized versions begin that way.

* `space-after`  
  The amount of space between printed objects, typically `12pt`.

### The `text` Element (Inside `pageParagraph`)

```
text[pageParagraph_text] :type=(title | text) => TEXT
```

This `text` element is nested inside a `pageParagraph`.  There is a
[different `text` element that is nested inside a
`container`](#the-text-element-inside-container).

This element has the following attributes:

* `type`  
  Always `text`.

The element is either empty, or contains CDATA that holds XHTML text
with a root element of either `html` or `p`.  Text in the XHTML can
contain substitution variables. The following variables are
supported:[^1]

[^1]: The `&` characters are escaped as `&amp;`, that is, these are
    not XML entities, since XML entity names can't begin with `[`.

* `&[Date]`  
  `&[Time]`  
  The current date or time in the preferred format for the locale.

* `&[Head1]`  
  `&[Head2]`  
  `&[Head3]`  
  `&[Head4]`  
  First-, second-, third-, or fourth-level heading, respectively.

* <a name="pagetitle">`&[PageTitle]`</a>  
  `&[Заголовок страницы]`  
  `&[頁面標題]`  
  The page title.

* `&[Filename]`  
  Name of the output file.

* `&[Page]`  
  `&[Страница]`  
  `&[頁]`  
  The page number.

See [Embedded HTML](#embedded-html) for more information.

> The 23,000 SPV files in the corpus have only 17 unique instances of
`text` inside `pageParagraph`.  Most of them look similar to this for
page headers:
>
> ```
> &lt;html xmlns="http://xml.spss.com/spss/viewer/viewer-tree">
>   &lt;head>
>
>   &lt;/head>
>   &lt;body>
>     &lt;p style="text-align:center; margin-top: 0">
>       &amp;[PageTitle]
>     &lt;/p>
>   &lt;/body>
> &lt;/html>
> ```
>
> and footers:
>
> ```
> &lt;html xmlns="http://xml.spss.com/spss/viewer/viewer-tree">
>   &lt;head>
>
>   &lt;/head>
>   &lt;body>
>     &lt;p style="text-align:right; margin-top: 0">
>       Page &amp;[Page]
>     &lt;/p>
>   &lt;/body>
> &lt;/html>
> ```
>
> Sometimes CSS is present (the original was indented much deeper), with
> header:
>
> ```
> &lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
>   &lt;head>
>           &lt;style type="text/css">
>                   p { font-family: sans-serif;
>                        font-size: 10pt; text-align: center;
>                        font-weight: normal;
>                        color: #000000;
>                        }
>           &lt;/style>
>   &lt;/head>
>   &lt;body>
>           &lt;p>&amp;amp;[PageTitle]&lt;/p>
>   &lt;/body>
> &lt;/html>
> ```
>
> and footer:
>
> ```
> &lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
>   &lt;head>
>           &lt;style type="text/css">
>                   p { font-family: sans-serif;
>                        font-size: 10pt; text-align: right;
>                        font-weight: normal;
>                        color: #000000;
>                        }
>           &lt;/style>
>   &lt;/head>
>   &lt;body>
>           &lt;p>Page &amp;amp;[Page]&lt;/p>
>   &lt;/body>
> &lt;/html>
> ```
>
> No files in the corpus show any more sophisticated use of features
> than these examples.

## Embedded HTML

Structure XML contains embedded HTML in two contexts:

- The [`text` element inside `container`](#the-text-element-inside-container).

- The [`text` element inside
  `pageParagraph`](#the-text-element-inside-pageparagraph).

The use of HTML in both cases is similar.  These HTML documents use
only the following elements:

* `html`  
  Sometimes, the document is enclosed with `<html>`...`</html>`.

* `head`  
  The document often contains a `head` element.  It can be
  empty or it can contain a `style` element, in turn enclosing CSS
  within `<!--` and `-->`.  See [embedded CSS](#embedded-ccs), below,
  for details.

* `body`  
  The document often contains a `body` element that contains the
  content.

* `p`  
  The document often contains a `p` element that contains the content.
  [Inside `pageParagraph`](#the-text-element-inside-pageparagraph),
  the document can contain multiple paragraphs.  On the other hand,
  [inside `container`](#the-text-element-inside-container), if there
  are multiple paragraphs, which is rare, `<p>` after the first are
  simply treated as line breaks, as with `<br>`.

  The following attributes are observed:

  - `align`  
    With value `left`, `center`, or `right`.

  - `style`  
    With value `text-align:<align>; margin-top: 0`, where `<align>` is
    one of `left`, `center`, or `right`, or simply `margin-top: 0`.

* `br`  
  The HTML body often begins with a "break" tag and may contain them
  as well.

  Embedded HTML writes most tag names in lowercase but this one is
  usually in uppercase, as `<BR>`.

  > At the beginning of an HTML body, a single `<BR>` element should
  > be ignored, but only if capitalized; that is, `<br>` is not
  > ignored, and neither is `<BR>` following any text or any other
  > element.

* `b`  
  `i`  
  `u`  
  `strike`  
  Styling.

* `font`  
  The following attributes are observed:

  - `face`  
    A typeface, most often `Monospaced` or `SansSerif`.

  - `color`  
    One of the forms `#RRGGBB` or `rgb (R, G, B)`.

  - `size`  
    A number between 1 and 7 with the following meanings:

    | `size` |    Size |
    |-------:|--------:|
    |  1[^2] |    6 pt |
    |      2 |  7.5 pt |
    |      3 |    9 pt |
    |      4 | 10.5 pt |
    |      5 | 13.5 pt |
    |      6 |   18 pt |
    |      7 |   27 pt |

    [^2]: This `size` doesn't appear in the corpus.  The size listed
    is an extrapolation based on what browsers usually do.

> It appears that pasting HTML into the SPSS viewer can cause more
> general HTML to be included.  The following elements in the corpus,
> each of these is observed in only a few files, appear to be added by
> pasting HTML from another application:
>
> * `strong`  
>   `em`  
>   Styling.
>
> * `span`  
>   The `style` attribute is used a bit, but not for CSS properties
>   that PSPP supports.
>
> * `li`  
>   `ul`  
>   Seen in only one file in the corpus.
>
> * `a`  
>   Seen in only two files in the corpus.  SPSS doesn't allow the link
>   to be seen or visited.
>
> * `table`  
>   `td`  
>   `tr`  
>   Seen in only one file in the corpus.  SPSS doesn't render the
>   table properly.
>
> * `img`  
>   Seen in only one file in the corpus.  In this file, the `src`
>   attribute was an invalid `jar:` URL.

Text in embedded HTML often uses non-breaking spaces (U+00A0
NON-BREAKING SPACE), often written as `&#160;` or `&nbsp;`.  In
embedded HTML, newlines must be treated as line breaks.

### Embedded CSS

The CSS in the corpus is simple.  To understand it, a parser only
needs to be able to skip white space, `<!--`, and `-->`, and parse style
only for `p` elements.  Only the following properties matter:

* `color`  
  In the form `RRGGBB`, e.g.  `000000`, with no leading `#`.

* `font-weight`  
  Either `bold` or `normal`.

* `font-style`  
  Either `italic` or `normal`.

* `text-decoration`  
  Either `underline` or `normal`.

* `font-family`  
  A font name, commonly `Monospaced` or `SansSerif`.

* `font-size`  
  Values claim to be in points, e.g. `14pt`, but the values are
  actually in "device-independent pixels" (px), at 96/inch.

### Examples

Text that looks like "plain **bold** *italic* ~~strikeout~~", for use
[inside `pageParagraph`]:

```
&lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p>
      plain&amp;#160;&lt;font color="#000000" size="3" face="Monospaced">&lt;b>bold&lt;/b>&lt;/font>&amp;#160;&lt;font color="#000000" size="3" face="Monospaced">&lt;i>italic&lt;/i>&amp;#160;&lt;strike>strikeout&lt;/strike>&lt;/font>
    &lt;/p>
  &lt;/body>
&lt;/html>
```

Another example, also for use [inside `pageParagraph`], of three
paragraphs, the first left justified, the second center justified with
a large font, and the third right justified:

```
&lt;html xmlns="http://www.w3.org/1999/xhtml" lang="en">
  &lt;head>

  &lt;/head>
  &lt;body>
    &lt;p>left&lt;/p>
    &lt;p align="center">&lt;font color="#000000" size="5" face="Monospaced">center&amp;#160;large&lt;/font>&lt;/p>
    &lt;p align="right">&lt;font color="#000000" size="3" face="Monospaced">&lt;b>&lt;i>right&lt;/i>&lt;/b>&lt;/font>&lt;/p>
  &lt;/body>
&lt;/html>
```

[inside `pageParagraph`]: #the-text-element-inside-pageparagraph
[inside `container`]: #the-text-element-inside-container

