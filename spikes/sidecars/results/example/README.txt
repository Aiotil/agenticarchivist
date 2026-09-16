About this folder
=================

This collection was catalogued with AgenticArchivist, but it does not need
AgenticArchivist to be used. Everything here is in open, standard formats.

Originals
  The image files in the collection folders are the originals. They are never
  modified by AgenticArchivist.

Sidecars (*.xmp next to each original)
  XMP metadata (ISO 16684-1) using Dublin Core, IPTC Photo Metadata, and an
  AgenticArchivist namespace (https://agenticarchivist.com/ns/xmp/1.0/).
  Adobe Lightroom, Adobe Bridge, digiKam, darktable, and ExifTool can read them.
  When two originals share a name (IMG_0600.tif and IMG_0600.jpg), the sidecar
  keeps the full name: IMG_0600.tif.xmp.

_agenticarchivist/works.vra.xml
  The catalogue in VRA Core 4 XML, including which images belong to each work.

_agenticarchivist/catalog.csv and catalog.json
  The same catalogue as a spreadsheet (one row per image) and as JSON.

_agenticarchivist/manifest-sha256.txt
  SHA-256 checksums of every original and sidecar. To check for damage, open a
  terminal in the collection folder and run:
      shasum -a 256 -c _agenticarchivist/manifest-sha256.txt

_agenticarchivist/derived/
  Previews and AI restorations. Restored images are marked as AI-altered in
  their embedded metadata (IPTC Digital Source Type).
