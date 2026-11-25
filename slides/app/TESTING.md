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
`status`, `author` (who signs new comments), `fonts` (the families DeckCraft can draw with:
Inter at once; Liberation Sans, Carlito and Caladea once fetched), `theme` (`mode`, `dark`, and
whether DeckCraft's half and egui are dark too).

Our own commands: `slides.new`, `slides.home`, `slides.rename {name}`, `slides.download {format:
pptx|pdf|outline}`, `slides.theme {mode: system|light|dark}`, `slides.textBox`, `slides.shape
{preset}`, `slides.presenter` (View › Presenter view: the show plus the presenter window).

Also in `state()`: `fullscreen`, `presenterView`, `perf` (DeckCraft's frame and render ms); and
`starslides.readyAt` (ms after the page started loading when the app was up), for speed checks.

## Slideshow and presenter view (browser)

- Slideshow (the button, F5, Ctrl+Enter) shows the slides alone, full screen. Firefox allows full
  screen only from inside the click or key press, so `src/fullscreen.rs` asks for it in that DOM
  event and starts the show itself; a show started by a menu or a script stays in the tab.
  Leaving full screen (Esc) ends the show.
- The presenter window opens for View › Presenter view, or by itself when the browser reports a
  second screen (`screen.isExtended`, Chromium). A blocked popup leaves an "Open presenter window"
  button on the page; ending the show removes it.
- The smoke test checks all of this in Firefox (headless). Chrome is not on the build machine.

DeckCraft's own page API, `window.deckcraft` (`execute`, `docInspect`, `uiInspect` …), is always
there as well.

## A test server

The throwaway Starbase test server serves this app at `/slides/` once the build is in its site
folder's `slides/` (the server serves any folder with an `index.html`, as `/docs/`).
