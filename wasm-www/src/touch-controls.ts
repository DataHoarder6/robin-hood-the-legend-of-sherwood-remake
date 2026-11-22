import { appendLogLine } from "./log.ts";
import type { RobinRpc } from "./replay.ts";

const INSTALLED_ATTRIBUTE = "data-touch-controls-installed";
const installed = new WeakMap<Element, () => void>();

/** On-screen buttons emit the same typed `PlayerCommand` JSON as the
 * keyboard/mouse path. No gameplay rule lives in the shell. */
export function installTouchControls(
  root: Pick<Document, "querySelector" | "querySelectorAll">,
  rpc: RobinRpc,
  touchEnabled: boolean,
): () => void {
  const container = root.querySelector<HTMLElement>("#touch-controls");
  if (container === null) return () => {};
  const previous = installed.get(container);
  if (previous !== undefined) return previous;

  const bound: [HTMLButtonElement, () => void][] = [];
  for (const button of container.querySelectorAll<HTMLButtonElement>(
    "button[data-robin-command]",
  )) {
    const raw = button.getAttribute("data-robin-command");
    if (raw === null) continue;
    let command: unknown;
    try {
      command = JSON.parse(raw) as unknown;
    } catch {
      continue;
    }
    const handler = (): void => {
      if (!touchEnabled) return;
      void rpc("command", command).catch((error: unknown) => {
        const log = root.querySelector<HTMLElement>("#log");
        const message = `[touch control failed] ${String(error)}`;
        if (log !== null) appendLogLine(log, message, "err");
        // eslint-disable-next-line no-console
        else console.error(message);
      });
    };
    button.addEventListener("click", handler);
    bound.push([button, handler]);
  }

  if (touchEnabled) container.removeAttribute("hidden");
  else container.setAttribute("hidden", "");
  container.setAttribute(INSTALLED_ATTRIBUTE, "true");

  const dispose = (): void => {
    for (const [button, handler] of bound) button.removeEventListener("click", handler);
    container.setAttribute("hidden", "");
    container.removeAttribute(INSTALLED_ATTRIBUTE);
    installed.delete(container);
  };
  installed.set(container, dispose);
  return dispose;
}
