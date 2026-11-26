### Fixed
- Photos › People: faces are sharp. Each face picture is now cut from the big (1024 px) thumbnail on the server and sent at 256 px, instead of being cut from the small 256 px thumbnail in the browser (a face 10% of a picture's width was ~38 px of image, now ~154 px).
- Photos › People: new people in later scans now show up as new, unnamed groups. Faces matched to a person automatically were used to match further faces, so one borderline match pulled every later face of a new person into an existing one. Only faces someone confirmed are compared with now, automatic matches are looked at again, and a group of new faces that as a whole isn't like a person stays its own group (the person is offered instead).

### Added
- Photos › People: rename from the list. Click a card's name (or "Add a name"), type, Enter or click elsewhere saves, Esc cancels.
