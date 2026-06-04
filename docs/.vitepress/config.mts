import { defineConfig } from 'vitepress'

export default defineConfig({
  title: 'l0-cache',
  description: 'Lightweight CLI proxy for LLM token savings',
  base: '/l0-cache/',
  themeConfig: {
    nav: [
      { text: 'Guide', link: '/guide/' },
      { text: 'Reference', link: '/reference/' },
      { text: 'GitHub', link: 'https://github.com/fabriziosalmi/l0-cache' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/' },
          { text: 'Installation', link: '/guide/installation' },
          { text: 'Quick Start', link: '/guide/quickstart' },
          { text: 'Configuration', link: '/guide/configuration' },
        ]
      },
      {
        text: 'Internals',
        items: [
          { text: 'Architecture', link: '/internals/architecture' },
          { text: 'Filter Pipeline', link: '/internals/filter-pipeline' },
          { text: 'Hardening', link: '/internals/hardening' },
          { text: 'Cross-Platform', link: '/internals/cross-platform' },
        ]
      },
      {
        text: 'Reference',
        items: [
          { text: 'CLI Options', link: '/reference/' },
          { text: 'Metrics Format', link: '/reference/metrics' },
          { text: 'Exit Codes', link: '/reference/exit-codes' },
        ]
      }
    ],
    socialLinks: [
      { icon: 'github', link: 'https://github.com/fabriziosalmi/l0-cache' }
    ]
  }
})
