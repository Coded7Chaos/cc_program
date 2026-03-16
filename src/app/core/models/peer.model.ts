export type PeerKind = 'App' | 'NonApp';

export interface PeerEntry {
  peer_id: string;
  hostname: string;
  ip: string;
  tcp_port: number;
  kind: PeerKind;
  last_seen: number;
  online: boolean;
  app_version?: string;
}
