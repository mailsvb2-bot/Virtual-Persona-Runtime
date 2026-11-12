"use strict";
// A tiny classic script runs independently of the Owner Lab module graph.
// If a browser cannot import app.js, the operator must not stare at "Загрузка…"
// indefinitely. Do not retry provider requests or change authorization here.
(() => {
  const status = document.getElementById("status");
  if (!status) return;
  const initialText = status.textContent?.trim() ?? "";
  if (initialText !== "Загрузка…") return;
  const deadlineMs = 12000;
  window.setTimeout(() => {
    // A late, legitimate bootstrap may still replace this error. Do not
    // overwrite a running or already failed application with false success.
    if (status.textContent?.trim() !== initialText) return;
    status.dataset.state = "error";
    status.textContent = "Интерфейс не загрузился. Проверьте подключение и обновите страницу.";
    const retry = document.createElement("button");
    retry.type = "button";
    retry.textContent = "Повторить загрузку";
    retry.addEventListener("click", () => window.location.reload());
    status.appendChild(document.createTextNode(" "));
    status.appendChild(retry);
  }, deadlineMs);
})();
