# agents docs site

Astro + Starlight + Vite. The content collection is a symlink to the repo-root
`docs/` directory, so:

- Markdown files live in `../docs/` and render natively when clicked on GitHub.
- Starlight reads the same files at build time — single source of truth.
- Pushes that touch `docs/` or `site/` trigger the GitHub Pages deploy workflow.

## Local development

```bash
cd site
pnpm install --frozen-lockfile
pnpm dev          # http://localhost:4321
pnpm build        # static output in site/dist
pnpm preview      # serve the built output
```

## Deploy

The `.github/workflows/deploy-docs.yml` workflow at the repo root builds `site/`
and publishes `site/dist/` to GitHub Pages on every push to `main` (or `master`)
that touches `docs/` or `site/`.

## What you (Brad) need to click in the GitHub UI before the first deploy works

The workflow itself is committed and ready, but Pages requires opt-in once per
repo. Two clicks:

1. **Repo → Settings → Pages → Build and deployment → Source** → choose
   **GitHub Actions** (not "Deploy from a branch"). This is what tells GitHub
   Pages to listen for the artifact our workflow uploads.

2. **Repo → Settings → Actions → General → Workflow permissions** — confirm
   that **Read and write permissions** is selected (or at minimum "Read
   repository contents and packages permissions" + "Allow GitHub Actions to
   create and approve pull requests" are off; we only need write for Pages).
   The workflow's `permissions:` block already declares `pages: write` and
   `id-token: write`, but the *org-/repo-level* setting must allow it.

Once those are set, push a commit that touches `docs/` or `site/` (or click
**Run workflow** on the *Deploy docs to GitHub Pages* action) and the site
appears at `https://<your-username>.github.io/<repo-name>/`.

## Why a symlink?

The user's directive was: "the markdown files under `docs/` so when people
click 'docs' on GitHub they see them, and they're auto-synced to Astro
Starlight." A symlink at `site/src/content/docs` → `../../../docs/` satisfies
both constraints with zero duplication and zero build-time copying. CI follows
the symlink because `actions/checkout@v4` preserves it.

## Theme

Customizations live in `src/styles/`:

- `tokens.css` — Arcflo design tokens (Geist + Geist Mono fonts, oklch
  monochrome scale, single electric-blue accent).
- `overrides.css` — maps the tokens onto Starlight's `--sl-*` variables and
  adds a few component-level rules (h2 top borders, sidebar active indicator,
  inline code chip styling).
