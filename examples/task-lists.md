# Task lists

List items starting with `[ ]` or `[x]` show a checkbox in place of the bullet. Checking or
unchecking one saves the change to this file. Run `mdreader examples` and open this file to try
it.

## Checked and unchecked

- [x] Parse task list markers
- [x] Draw checkboxes in place of bullets
- [ ] Write the release notes

## Mixed with plain items

Task items and plain items can share a list, and their text stays aligned.

- [x] Support light and dark themes
- Support mermaid diagrams
- [ ] Support math

## Nested lists

- [ ] Ship version 0.2
  - [x] Auto refresh
  - [x] Jump to headings
  - [ ] Task lists
    - [x] Render checkboxes
    - [ ] Add an example

## Loose lists

A blank line between items puts each item in a paragraph. The checkbox stays in place, and later
paragraphs line up with the item's text.

- [x] Review the design document

  Comments are resolved in the document.

- [ ] Announce the release

## Ordered lists

Ordered lists keep their numbers, with the checkbox after the number.

1. [x] Build the binary
2. [x] Run the tests
3. [ ] Tag the release
