// Type side-car for scripts/verify-cloudflare-deployment.mjs. The verifier runs
// directly under Node (deployment suites import the .mjs at runtime), so its
// public surface is declared here for TypeScript consumers. Keep this in sync
// with the export list in verify-cloudflare-deployment.mjs.

export interface Deployment {
  readonly apiBase: string;
  readonly publicHost: string;
  readonly publicOrigin: string;
  readonly publicWorker: string;
  readonly datadirWorker: string;
  readonly runtimeWorker: string;
  readonly signerOrigin: string;
  readonly signerHost: string;
  readonly signerWorker: string;
  readonly wranglerVersion: string;
  readonly zoneName: string;
}

export declare const DEPLOYMENT: Deployment;

export declare const EXPECTED_PUBLIC_ROUTES: ReadonlyArray<
  Readonly<{ pattern: string; script: string | null }>
>;

export declare const PUBLIC_ISOLATION: Readonly<Record<string, string>>;
export declare const SAME_ORIGIN_RESOURCE: string;
export declare const SIGNER_ISOLATION: Readonly<Record<string, string>>;

export interface WranglerAssets {
  directory: string;
  html_handling: string;
  not_found_handling: string;
}

export interface WranglerConfig {
  $schema: string;
  name: string;
  compatibility_date: string;
  workers_dev: boolean;
  preview_urls: boolean;
  assets: WranglerAssets;
  routes?: ReadonlyArray<{ pattern: string; custom_domain: boolean }>;
}

export interface PublicRoutes {
  schema_version: number;
  zone_name: string;
  worker_name: string;
  routes: ReadonlyArray<{ pattern: string; script: string | null }>;
}

export interface DeploymentSnapshot {
  packageJson: { devDependencies?: { wrangler?: string } };
  publicWrangler: WranglerConfig;
  runtimeWrangler: WranglerConfig;
  datadirWrangler: WranglerConfig;
  signerWrangler: WranglerConfig;
  routes: PublicRoutes;
  publicHeaders: string;
  runtimeHeaders: string;
  datadirHeaders: string;
  signerHeaders: string;
}

export declare function validateDeploymentSnapshot(snapshot: DeploymentSnapshot): void;
export declare function validatePublicHeaders(text: string): void;
export declare function validateSignerHeaders(text: string): void;
export declare function validateRuntimeHeaders(text: string): void;
export declare function validateDatadirHeaders(text: string): void;
export declare function loadDeploymentSnapshot(root?: string): Promise<DeploymentSnapshot>;
export declare function verifyDeploymentConfig(root?: string): Promise<void>;
