import { defineConfig } from 'vitepress'

export default defineConfig({
  head: [
    // Tutto first-party. 'unsafe-inline' serve perche' VitePress emette
    // uno script inline per il tema e stili inline.
    [
      'meta',
      {
        'http-equiv': 'Content-Security-Policy',
        content:
          "default-src 'self'; script-src 'self' 'unsafe-inline'; " +
          "style-src 'self' 'unsafe-inline'; img-src 'self' data:; " +
          "font-src 'self'; connect-src 'self'; base-uri 'self'; form-action 'self'",
      },
    ],
  ],
  title: 'l0-compressor',
  description: 'Lightweight CLI proxy for LLM token savings',
  base: '/l0-compressor/',
  themeConfig: {
    footer: {
      message:
        '<a href="https://fabriziosalmi.github.io/privacy">Privacy &amp; legal</a>',
    },
    nav: [
      { text: 'Guide', link: '/guide/' },
      { text: 'Reference', link: '/reference/' },
      { text: 'GitHub', link: 'https://github.com/fabriziosalmi/l0-compressor' }
    ],
    sidebar: [
      {
        text: 'Guide',
        items: [
          { text: 'Introduction', link: '/guide/' },
          { text: 'Installation', link: '/guide/installation' },
          { text: 'Quick Start', link: '/guide/quickstart' },
          { text: 'Configuration', link: '/guide/configuration' },
          { text: 'Claude Code Integration', link: '/guide/claude-code' },
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
      { icon: 'github', link: 'https://github.com/fabriziosalmi/l0-compressor' }
    ]
  }
})
