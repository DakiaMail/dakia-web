// Installed before React starts in the standalone sandbox build only.
// The normal app has no local transport and retains its server behavior.
export interface LocalTransport {
  download(account: string, id: string): string;
}
export let localTransport: LocalTransport | undefined;
export function setLocalTransport(transport: LocalTransport) {
  localTransport = transport;
}
