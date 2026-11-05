// Keep in sync with cloud_verifier::OFFICIAL_LEGACY_IDS. The backend remains
// authoritative and validates scope again when a release is submitted.
const officialLegacyIds = new Set([
  'audio.play.process', 'hotkeys', 'l4d2.interactive', 'minecraft.server',
  'peak.game', 'repo.game', 'sonicboom.server', 'textintel',
])

export function publisherAllowsPlugin(namespace: string, pluginId: string): boolean {
  return (pluginId.startsWith(`${namespace}.`) && pluginId.length > namespace.length + 1)
    || (namespace === 'tiktools' && officialLegacyIds.has(pluginId))
}
