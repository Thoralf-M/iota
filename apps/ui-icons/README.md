# `@iota/apps-ui-icons`

## Usage

Installation:

```bash
# Replace with npm, yarn, bun, etc.
pnpm install @iota/apps-ui-icons
```

Importing:

```typescript
import { IotaLogoMark } from '@iota/apps-ui-icons';
```

## Development

### Exporting Icons

1. Clean the project by running `pnpm icons clean:all` from the repo root.
1. Open Figma Icons file. Create a mass export using `cmd` + `shift` + `e`. Save the SVGs to `iota/apps/ui-icons/svgs`.
1. Run `pnpm icons generate` to generate the new icon library.
1. Commit the changes and submit a PR.
