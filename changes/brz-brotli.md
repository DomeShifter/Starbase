### Changed
- Smaller downloads: the web apps and fonts are now also shipped brotli-compressed (and gzip at level 9), and the server sends the smallest one the browser accepts. Fonts are served with the right type and revalidate cheaply, and files are streamed from disk.
