// @ts-check
import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';

// Read GitHub repo info from env so the action passes the right base path.
// Falls back to no-base for local dev.
const repoBase = process.env.AGENTS_BASE || '/';
const siteUrl = process.env.AGENTS_SITE_URL || 'https://example.github.io';

export default defineConfig({
  site: siteUrl,
  base: repoBase,
  trailingSlash: 'ignore',
  integrations: [
    starlight({
      title: 'agents',
      description: 'Multi-machine AI skills sync — Vercel-compatible CLI with invisible git auto-sync.',
      logo: {
        light: './src/assets/logo.svg',
        dark: './src/assets/logo.svg',
        replacesTitle: false,
      },
      social: {
        github: 'https://github.com/bradleykester/agents',
      },
      customCss: [
        './src/styles/tokens.css',
        './src/styles/overrides.css',
      ],
      sidebar: [
        {
          label: 'Start here',
          items: [
            { label: 'Installation', slug: 'install' },
            { label: 'Quickstart', slug: 'quickstart', badge: { text: '5 min', variant: 'tip' } },
          ],
        },
        {
          label: 'Concepts',
          autogenerate: { directory: 'concepts' },
        },
        {
          label: 'Reference',
          autogenerate: { directory: 'reference' },
        },
        {
          label: 'Operate',
          autogenerate: { directory: 'operate' },
        },
      ],
      lastUpdated: true,
      pagination: true,
    }),
  ],
});
