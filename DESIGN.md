# AutoForge UI additions

## Existing system
Source: `frontend/src/index.css`, `NewProjectPage`, `ProjectDetailPage`, `Layout`.
Use existing dark surfaces: bg #0b1326, card #131b2e, border #424753,
foreground #dae2fd, muted #c2c6d5, accent #5891ff, primary #afc6ff.
Use existing font-sans, font-headline and spacing (4, 8, 12, 16, 20, 24, 32px).
No new theme, dependency or motion system is required for these additions.
Print is black text on white A4 paper with 18mm vertical and 15mm horizontal margins.

## Layout and content
Layout: reuse the existing navigation and scrolling main region; estimate editor
is a centred, single-column form with two-column metadata on desktop and stacked
fields on mobile. Item rows use a descriptive field followed by quantity and price.
At narrow widths the navigation becomes a top row to preserve the editor width.
Print is ordinary document flow; table headings repeat, individual rows and totals
stay together, long notes may span pages.

Primary action: save estimate. Secondary actions: generate AI draft and print each
document kind. Priorities: metadata, requirements and daily rate, editable items
and total, assumptions and output actions. Required fields: title, customer,
supplier/contact, issue date, expiry, requirements, daily rate, item descriptions,
quantities, prices, VAT toggle, totals, notes. Saved estimates can be reopened.
AI effort estimates use the user's daily rate and remain editable.

Coding skills reuse the surrounding project fieldset and card styles, with three
checkboxes, optional Markdown text/file input, and a save action on detail pages.
Explain that saved changes affect subsequent stages and that linked scripts are
not installed. Do not claim measured token savings.

Left out: decorative metrics, sample prices, marketing badges and new UI libraries.

## Accessibility and verification
Label every input, use native keyboard controls, visible focus outlines, 44px
button targets and error/status live regions. Verify narrow widths and print
pagination through actual browser captures. Reuse the existing dark palette;
the application has no light theme. Record remaining verification limits in the
task report rather than claiming full WCAG conformance.
