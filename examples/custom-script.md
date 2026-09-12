# Custom scripts

`--script` adds a script of your own to every page, so it can draw elements that
mdreader does not know how to draw itself:

```sh
mdreader examples --script examples/custom-script.js
```

The script is loaded as an ES module, after the page's own script. Editing it
reloads the open pages, the same way editing a markdown file updates them.

## Rendering an element

Fenced code blocks carry a `lang-<name>` class taken from their info string, so
a script can find the ones it wants to draw. This page has a `chart` block, and
[`custom-script.js`](https://github.com/wingyplus/mdreader/blob/main/examples/custom-script.js)
draws it as a bar chart:

```chart
Rust: 62
Go: 41
Elixir: 28
Ruby: 15
```

That takes one renderer:

```js
mdreader.onRender((main, { theme }) => {
  for (const block of main.querySelectorAll("pre.lang-chart")) {
    block.innerHTML = drawChart(mdreader.source(block), theme);
    block.classList.add("rendered");
  }
});
```

## The API

`window.mdreader` has two functions.

- `onRender(renderer)` registers a renderer, and runs it. A renderer is called
  as `renderer(main, { theme, signal })`, where `main` is the element holding
  the page content, `theme` is `"light"` or `"dark"`, and `signal` is an
  [`AbortSignal`](https://developer.mozilla.org/en-US/docs/Web/API/AbortSignal).
  It may be async.

  Renderers run again after every page update and whenever the theme changes, so
  a renderer draws what it finds rather than assuming it runs once. It is called
  with the content already in the page, so an element can be measured.

  `signal` is aborted once a newer run supersedes this one. An async renderer
  should check `signal.aborted` after each `await` and stop, instead of drawing
  over newer content.

- `source(element)` gives the text the server sent for a `<pre>` block, which
  stays available after the block's content has been replaced. It is how a
  renderer reads a block again on a redraw, such as after a theme change.

Adding the `rendered` class to an element tells mdreader two things: the block
is drawn, so it drops the code block styling around it, and the drawing should
be kept while the page updates, instead of flashing back to its source until it
is drawn again.

Renderers registered by a `<script>` in the markdown itself work the same way.
Those run before the page's own script, so they are collected and run once it
is ready.
