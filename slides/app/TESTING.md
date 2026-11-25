# Testing Starbase Slides

AI-first: drive the app with commands and read its state as JSON, instead of clicking at pixel
positions. Screenshots are only for how it looks (check light and dark).

## Window tests (native, no browser)

`cargo test -p starslides` (from `slides/`, after `scripts/fetch-upstream.sh`) runs
`src/shell_tests.rs`: our menus replace DeckCraft's title bar, ribbon and status bar; every menu
command and icon exists; the toolbar runs DeckCraft commands; light and dark switch both halves;
File › Download gives a .pptx; a viewer gets a greyed toolbar.

## In the browser: `window.starslides`

Open the page with `test` in its address (`slides/?file=<id>&test`, or `?new=1&test`). Then:

```js
const n = starslides.run("slide.new", {});          // any DeckCraft command id, or ours (slides.*)
JSON.parse(await starslides.wait(n));                // {"ok": …} or {"err": "…"}
JSON.parse(await starslides.wait(starslides.state())) // the window and presentation
```

`state()` has: `shell` (title shown, save state "saving"/"saved"/"failed: …", server file name,
view only, theme), `documents` (title, unsaved changes, file name: always a .pptx on the server),
`slides` (count), `current`, `view`, `notes`, `pane`, `tool`, `format`, `showing` (slideshow on),
`status`, `theme` (`mode`, `dark`, and whether DeckCraft's half and egui are dark too).

Our own commands: `slides.new`, `slides.home`, `slides.rename {name}`, `slides.download {format:
pptx|pdf|outline}`, `slides.theme {mode: system|light|dark}`, `slides.textBox`, `slides.shape
{preset}`.

DeckCraft's own page API, `window.deckcraft` (`execute`, `docInspect`, `uiInspect` …), is always
there as well.

## A test server

The throwaway Starbase test server serves this app at `/slides/` once the build is in its site
folder's `slides/` (the server serves any folder with an `index.html`, as `/docs/`).
