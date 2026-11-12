# Theme compatibility

**No community theme has been tested.** This file describes what the CSS engine supports, so the
effect of a theme can be predicted; it is not a list of verified themes.

Themes are read from `.obsidian/themes/<name>/theme.css` (the name in `appearance.json` →
`cssTheme`), snippets from `.obsidian/snippets/<name>.css` (those in `enabledCssSnippets`). Order:
built-in base, theme, snippets. Files are reloaded when they change. `rustsidian doctor <vault>` lists
how many constructs in each enabled snippet were not understood.

## Supported CSS

| Area | Supported |
|---|---|
| Selectors | type, `.class`, `#id`, `*`, `[attr]`, `[attr=v]`, `^=`, `$=`, `*=`, `~=`; descendant, `>`, `+`, `~` |
| Pseudo-classes | `:hover` `:focus` `:active` `:disabled` `:checked` `:first-child` `:last-child` `:root` `:not(compound)` `:is()` `:where()` |
| Cascade | specificity, source order, later sheets win, `!important`, `inherit`, `initial`/`unset` |
| Custom properties | definition, inheritance, `var(--x, fallback)`, nested fallbacks, cycle detection |
| Values | hex, `rgb()/rgba()`, `hsl()/hsla()`, 46 named colors; `px em rem % pt vw vh`; `calc() min() max() clamp()` |
| At-rules | `@media (prefers-color-scheme: dark|light)`, `@media (prefers-reduced-motion: reduce)` |
| Shorthands | `margin`, `padding`, `border`, `border-<side>`, `border-width/color/style`, `background` (color only), `gap`, `overflow` |
| Layout properties read by the UI | `display` (flex, block, none), `flex-direction`, `flex-grow`, `flex-shrink`, `width`, `height`, min/max sizes, `gap`, `align-items`, `justify-content`, `position: absolute` with insets, `overflow`, `opacity` |
| Paint properties read by the UI | `background-color`, `color`, border widths and colors, `border-radius`, `font-size`, `font-weight`, `font-style`, `line-height`, `white-space`, `text-align: center`, `font-family` (only to choose monospace or not) |

## Not supported

Pseudo-elements (`::before`, `::after`), `:nth-child()`, `:has()`, `@supports`, `@layer`,
`@container`, width/height media queries, `@font-face`, `@import`, `@keyframes`, gradients,
background images, shadows, transforms, filters, grid, `color-mix()`. `transition` is parsed but
not animated. Rules using an unsupported selector are skipped as a whole.

## Obsidian names the base theme defines

Body classes `theme-dark` / `theme-light`. Variables: `--background-primary`,
`--background-primary-alt`, `--background-secondary`, `--background-secondary-alt`,
`--background-modifier-border`, `--background-modifier-hover`, `--text-normal`, `--text-muted`,
`--text-faint`, `--text-accent`, `--text-on-accent`, `--text-highlight-bg`, `--text-selection`,
`--interactive-accent`, `--accent-h/s/l`, `--code-background`, `--code-normal`, `--tag-color`,
`--font-text-size`, `--font-ui-small`, `--font-ui-smaller`, `--h1-size` … `--h6-size`, `--radius-s`,
`--radius-m`. Element classes: `workspace`, `workspace-ribbon`, `side-dock-ribbon-action`,
`workspace-split` with `mod-left-split` / `mod-right-split` / `mod-root`, `workspace-tab-header`,
`view-header`, `view-content`, `markdown-source-view`, `markdown-reading-view`, `cm-editor`,
`cm-line`, `nav-files-container`, `nav-file-title`, `nav-folder-title`, `status-bar`,
`status-bar-item`, `prompt`, `prompt-input`, `suggestion-item`, `is-active`, `is-selected`.

Obsidian defines several hundred variables and many more classes; a theme that targets ones not
listed here has no effect on those parts.
