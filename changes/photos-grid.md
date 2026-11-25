### Added
- Starbase Drive: "View as photos" shows a folder as a grid of pictures, with a full-screen
  viewer (arrow keys, swipe, zoom) and a slideshow (every 3, 5 or 10 seconds, in order or
  shuffled). It works in the browser, and on the desktop when connected to a server. Photos is
  off until someone who can edit the folder turns it on for that folder.
- Server: an index of the pictures in the folders where Photos is on, with small and large
  previews made in the background at low priority (docs/photos-api.md). The folder's owner can
  turn it off again, which deletes those previews.
