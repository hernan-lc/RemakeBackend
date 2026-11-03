// jsdom lacks matchMedia; reka-ui / components may probe it.
if (typeof window !== 'undefined' && !window.matchMedia) {
  window.matchMedia = () => ({
    matches: false,
    media: '',
    addEventListener: () => {},
    removeEventListener: () => {},
  }) as unknown as MediaQueryList
}
