# Mermaid diagrams

Fenced code blocks tagged `mermaid` are drawn as diagrams, following the page theme. Run
`mdreader examples` and open this file to see them rendered.

## Flowchart

```mermaid
flowchart TD
    A[Fenced code block] --> B{Language?}
    B -->|mermaid| C[Draw diagram in the browser]
    B -->|supported| D[Highlight with tree-sitter]
    B -->|other| E[Render as plain text]
```

## Sequence diagram

```mermaid
sequenceDiagram
    participant Browser
    participant mdreader
    participant Disk
    Browser->>mdreader: GET /guide.md
    mdreader->>Disk: Read guide.md
    Disk-->>mdreader: Markdown
    mdreader-->>Browser: HTML page
    Browser->>Browser: Render mermaid diagrams
```

## Class diagram

```mermaid
classDiagram
    class Node {
        <<enumeration>>
        Dir
        File
    }
    class Dir {
        +String name
        +Vec~Node~ children
    }
    class File {
        +String name
        +String path
    }
    Node <|-- Dir
    Node <|-- File
    Dir o-- Node : children
```

## State diagram

```mermaid
stateDiagram-v2
    [*] --> System
    System --> Light : toggle
    Light --> Dark : toggle
    Dark --> System : toggle
```

## Entity relationship diagram

```mermaid
erDiagram
    DIRECTORY ||--o{ DIRECTORY : contains
    DIRECTORY ||--o{ MARKDOWN_FILE : contains
    MARKDOWN_FILE ||--o{ CODE_BLOCK : has
```

## Gantt chart

```mermaid
gantt
    title Writing a design document
    dateFormat YYYY-MM-DD
    axisFormat %b %d
    section Draft
    Outline      :done, outline, 2026-09-01, 3d
    First draft  :done, draft, after outline, 5d
    section Review
    Team review  :active, review, after draft, 4d
    Revisions    :revise, after review, 3d
    section Publish
    Published    :milestone, after revise, 0d
```

## Pie chart

```mermaid
pie title Languages in a sample project
    "Shell" : 45
    "Rust" : 30
    "Mermaid" : 15
    "Other" : 10
```

## Invalid diagram

A diagram with a syntax error is left as its source text, and the error is logged to the browser
console.

```mermaid
flowchart TD
    A -->
```
