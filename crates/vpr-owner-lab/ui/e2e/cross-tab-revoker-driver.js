(() => {
  const mailbox = "/__journey/report/expressive";
  const sleep = (ms) => new Promise((r) => window.setTimeout(r, ms));
  const waitFor = async (predicate, phase) => {
    const start = performance.now();
    while (performance.now() - start < 25_000) {
      if (await predicate()) return;
      await sleep(20);
    }
    throw new Error("NON_CONNECTING_TAB_TIMEOUT:" + phase);
  };
  const get = (id) => {
    const e = document.getElementById(id);
    if (!e) throw new Error("SECOND_TAB_MISSING_CONTROL:" + id);
    return e;
  };
  const send = async (report) => {
    await fetch(mailbox, {
      method: "POST",
      headers: { "Content-Type": "text/plain;charset=UTF-8" },
      cache: "no-store",
      body: JSON.stringify(report),
    });
  };
  const run = async () => {
    try {
      // This tab never uses Connect; it observes the owner session created
      // in tab A and must still broadcast that exact canonical session ID.
      await waitFor(
        () => (get("status").textContent ?? "").includes("Найдена незакрытая сессия"),
        "existing-session-discovered",
      );
      const revoke = get("revoke");
      await waitFor(() => !revoke.disabled, "revoke-enabled");
      revoke.click();
      await waitFor(
        () => (get("status").textContent ?? "").includes("Доступ отозван."),
        "revoke-completed",
      );
      await send({ kind: "phase", phase: "non-connecting-tab-revoked" });
    } catch (error) {
      await send({
        status: "failed",
        error: "SECOND_TAB:" + (error instanceof Error ? error.message : String(error)),
      }).catch(() => undefined);
    }
  };
  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", () => void run(), { once: true });
  } else {
    void run();
  }
})();
