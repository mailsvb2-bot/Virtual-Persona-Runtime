export type BootstrapContext = { csrfToken: string };

let resolveBootstrap!: (context: BootstrapContext) => void;
const bootstrapReady = new Promise<BootstrapContext>((resolve) => {
  resolveBootstrap = resolve;
});
let published = false;

export const publishBootstrap = (csrfToken: string): void => {
  if (published) return;
  const normalized = csrfToken.trim();
  if (!normalized) throw new Error("BOOTSTRAP_CSRF_MISSING");
  published = true;
  resolveBootstrap({ csrfToken: normalized });
};

export const whenBootstrap = (): Promise<BootstrapContext> => bootstrapReady;
