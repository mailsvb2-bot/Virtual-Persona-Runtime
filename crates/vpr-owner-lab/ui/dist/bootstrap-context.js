let resolveBootstrap;
const bootstrapReady = new Promise((resolve) => {
    resolveBootstrap = resolve;
});
let published = false;
export const publishBootstrap = (csrfToken) => {
    if (published)
        return;
    const normalized = csrfToken.trim();
    if (!normalized)
        throw new Error("BOOTSTRAP_CSRF_MISSING");
    published = true;
    resolveBootstrap({ csrfToken: normalized });
};
export const whenBootstrap = () => bootstrapReady;
