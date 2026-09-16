# Frontend quality requirements

- UI quality and consistency are required for every feature, not a separate optional pass.
- Reuse the existing cloud page header, tabs, panel header/body, typography, spacing,
  color and button conventions. Do not invent a separate visual language per page.
- Use `Panel`, `PanelBody`, `FieldActionRow` and `ConfigPreview` from `cloud/ui.tsx`
  for new detail surfaces. YAML uses `ConfigPreview`/`CodeBlock`, never a bespoke
  bare `pre`. Extend shared components when needed instead of duplicating styling.
- Card containers stay static on hover: no lifting, hover shadows or border animation.
  Preserve keyboard focus indicators and useful link/button feedback.
- Separate reading, editing and version-management tasks. Show core content first;
  put long rules, provenance and advanced actions in clearly labelled tabs/sections.
- Avoid double-padding panel headers, unbounded full-width form fields, dense stacks
  of unspaced paragraphs, redundant explanations and raw hashes dominating the page.
- Detail tabs and version selection must survive refresh via URL state. Installed
  configuration must expose readable, copyable source; distinguish parameterized
  fragments from final identity output.
- Before release, run build/lint and inspect real desktop and mobile screenshots.
  Exercise affected navigation, loading/error states and primary actions in a browser.
  Check keyboard focus and overflow. Fix visual defects before declaring completion.
