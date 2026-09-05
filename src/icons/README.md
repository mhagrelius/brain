Single-fill symbolic SVGs (these two are Adwaita, GPL / CC-BY-SA). The icon
provider rewrites the `fill` at render time, so ship one copy per icon and ask
for `image://icon/<name>/<#hex>` from QML via `Icon { name: "folder" }`.
Strip `<metadata>` blocks before adding one; they are most of the file size.
