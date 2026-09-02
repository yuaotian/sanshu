export type RerankerProxyType = 'http' | 'socks5'

export interface RerankerProxySettings {
  proxy_type: RerankerProxyType
  host: string
  port: number
}

export interface DetectedRerankerProxy extends RerankerProxySettings {
  response_time_ms: number | null
}

export type RerankerDownloadNetwork =
  | { mode: 'direct' }
  | { mode: 'proxy', proxy: RerankerProxySettings }

export interface RerankerRouteProbe {
  mode: 'direct' | 'proxy'
  label: string
  available: boolean
  supports_ranges: boolean
  completed_samples: number
  total_samples: number
  median_bytes_per_second: number | null
  min_bytes_per_second: number | null
  max_bytes_per_second: number | null
  variation_percent: number | null
  median_ttfb_ms: number | null
  error: string | null
}

export interface RerankerDownloadProbeResult {
  direct: RerankerRouteProbe
  proxy: RerankerRouteProbe | null
}

export interface RerankerDownloadSelection {
  network: RerankerDownloadNetwork
  remember_proxy: boolean
}
