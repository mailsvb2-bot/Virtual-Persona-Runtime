type TtsRoutes = { active: string | null; alternate: string | null };
type Bootstrap = { csrf_token: string };

const panel = document.getElementById("tts-switch-panel");
const selector = document.getElementById("tts-switch-provider") as HTMLSelectElement | null;
const submit = document.getElementById("tts-switch-apply") as HTMLButtonElement | null;
const feedback = document.getElementById("tts-switch-status");

async function jsonGet<T>(route: string): Promise<T> {
  const response = await fetch(route, { cache: "no-store", credentials: "same-origin" });
  if (!response.ok) throw new Error("TTS settings unavailable");
  return await response.json() as T;
}

async function mountTtsSelector(): Promise<void> {
  if (!panel || !selector || !submit || !feedback) return;
  let routes: TtsRoutes;
  let bootstrap: Bootstrap;
  try {
    [routes, bootstrap] = await Promise.all([
      jsonGet<TtsRoutes>("/api/tts/providers"),
      jsonGet<Bootstrap>("/api/bootstrap"),
    ]);
  } catch {
    return;
  }
  if (!routes.active || !routes.alternate || !bootstrap.csrf_token) return;
  for (const name of [routes.active, routes.alternate]) {
    const option = document.createElement("option");
    option.value = name;
    option.textContent = name;
    selector.append(option);
  }
  selector.value = routes.active;
  panel.hidden = false;
  feedback.textContent = "Смена применяется к следующей реплике, без смены сеанса аватара.";
  submit.addEventListener("click", async () => {
    if (!selector.value || submit.disabled) return;
    submit.disabled = true;
    try {
      const response = await fetch("/api/tts/select", {
        method: "POST",
        credentials: "same-origin",
        headers: {
          "Content-Type": "application/json",
          "X-VPR-CSRF": bootstrap.csrf_token,
        },
        body: JSON.stringify({ provider: selector.value }),
      });
      if (!response.ok) {
        feedback.textContent = response.status === 409
          ? "Дождитесь завершения текущей реплики и повторите переключение."
          : "Переключение отклонено сервером.";
        return;
      }
      const current = await response.json() as TtsRoutes;
      feedback.textContent = "Теперь активен: " + (current.active ?? "неизвестно");
    } catch {
      feedback.textContent = "Не удалось связаться с локальным сервером.";
    } finally {
      submit.disabled = false;
    }
  });
}

void mountTtsSelector();
