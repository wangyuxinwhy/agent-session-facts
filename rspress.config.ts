import { defineConfig } from '@rspress/core';

export default defineConfig({
  root: 'docs',
  base: '/agent-session-facts/',
  siteOrigin: 'https://wangyuxinwhy.github.io',
  ssg: true,
  llms: true,
  title: 'agent-session-facts',
  description:
    'Turn Codex, Pi, and Claude Code Session logs into auditable loop, model-usage, tool-call, cost, and source-coverage facts.',
  route: {
    cleanUrls: true,
  },
  themeConfig: {
    llmsUI: {
      injectLlmsHint: true,
      viewOptions: ['markdownLink', 'chatgpt', 'claude'],
      placement: 'title',
    },
    nav: [
      { text: 'Guide', link: '/guide/installation' },
      { text: 'Schema', link: '/reference/schema' },
    ],
    sidebar: {
      '/guide/': [
        {
          text: 'Guide',
          items: [
            { text: 'Overview', link: '/guide/overview' },
            { text: 'Installation', link: '/guide/installation' },
            { text: 'Usage', link: '/guide/usage' },
          ],
        },
      ],
      '/reference/': [
        {
          text: 'Reference',
          items: [{ text: 'Output Schema', link: '/reference/schema' }],
        },
      ],
    },
    socialLinks: [
      {
        icon: 'github',
        mode: 'link',
        content: 'https://github.com/wangyuxinwhy/agent-session-facts',
      },
    ],
  },
});
