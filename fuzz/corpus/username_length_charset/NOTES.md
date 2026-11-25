# Seed corpus notes (Issue #848)

Each file is `[path_selector][username bytes]`. Even path selector exercises
`register`; odd exercises `register_sponsored`.

- `empty`: selector + empty username -> InvalidUsername before auth or writes.
- `max39`: selector + 39 ASCII alphanumerics -> accepted at the length boundary.
- `too_long40`: selector + 40 ASCII alphanumerics -> InvalidUsername.
- `non_ascii`: selector + UTF-8 accented character -> InvalidUsername.
- `bad_charset`: selector + underscore-wrapped name and consecutive hyphens -> InvalidUsername.
