export type TransferRole = 'Sender' | 'Receiver';
export type TransferStatus =
  | 'Pending'
  | 'InProgress'
  | 'Completed'
  | 'Cancelled'
  | { Failed: string };

export interface ActiveTransfer {
  transfer_id: string;
  file_name: string;
  file_path: string;
  file_size: number;
  total_chunks: number;
  chunk_size: number;
  destination_path: string;
  role: TransferRole;
  chunks_done: boolean[];
  status: TransferStatus;
  target_peers: string[];
  sender_ip: string;
  sender_peer_id: string;
  bytes_transferred: number;
  started_at: number;
}

export interface TransferProgressPayload {
  transfer_id: string;
  chunks_completed: number;
  total_chunks: number;
  bytes_transferred: number;
  speed_bps: number;
  status: TransferStatus;
}

export interface FileInfo {
  name: string;
  path: string;
  size: number;
  sha1: string;
}
