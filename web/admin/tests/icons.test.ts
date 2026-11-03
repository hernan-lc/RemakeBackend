import { describe, expect, it } from 'vitest'
import { mount } from '@vue/test-utils'
import AppIcon from '../src/components/icons/AppIcon.vue'
import { iconPaths, type IconName } from '../src/components/icons/icons'

describe('reusable SVG icon library', () => {
  it('exposes a non-empty path for every icon name', () => {
    const names = Object.keys(iconPaths) as IconName[]
    // The rail, header, and views depend on this breadth; shrinking the
    // set orphans a consumer.
    expect(names.length).toBeGreaterThan(20)
    for (const name of names) {
      expect(iconPaths[name], name).toMatch(/<(path|rect|circle|line|polyline|polygon)/)
    }
  })

  it('renders a decorative svg by default', () => {
    const wrapper = mount(AppIcon, { props: { name: 'dashboard' } })
    const svg = wrapper.find('svg')
    expect(svg.exists()).toBe(true)
    expect(svg.attributes('aria-hidden')).toBe('true')
    expect(svg.attributes('role')).toBeUndefined()
    expect(svg.attributes('width')).toBe('18')
  })

  it('renders a labelled image when a label is given', () => {
    const wrapper = mount(AppIcon, { props: { name: 'moon', label: 'Dark mode', size: 24 } })
    const svg = wrapper.find('svg')
    expect(svg.attributes('role')).toBe('img')
    expect(svg.attributes('aria-label')).toBe('Dark mode')
    expect(svg.attributes('width')).toBe('24')
  })
})
