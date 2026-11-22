export type CanvasEnvironment = {
  viewport: () => {
    readonly fullscreen: boolean;
    readonly innerWidth: number;
    readonly innerHeight: number;
    readonly devicePixelRatio: number;
    readonly insets?: {
      readonly top: number;
      readonly right: number;
      readonly bottom: number;
      readonly left: number;
    };
  };
  subscribe: (sync: () => void) => () => void;
};

const NO_INSETS = { top: 0, right: 0, bottom: 0, left: 0 } as const;

function browserEnvironment(canvas: HTMLCanvasElement): CanvasEnvironment {
  const inset = (side: "top" | "right" | "bottom" | "left"): number => {
    const view = canvas.ownerDocument.defaultView;
    const raw = view
      ?.getComputedStyle(canvas.ownerDocument.documentElement)
      .getPropertyValue(`--safe-${side}`)
      .trim();
    const parsed = raw === undefined || raw === "" ? 0 : Number.parseFloat(raw);
    return Number.isFinite(parsed) ? parsed : 0;
  };
  return {
    viewport: () => ({
      fullscreen: document.fullscreenElement === canvas,
      innerWidth: window.innerWidth,
      innerHeight: window.innerHeight,
      devicePixelRatio: window.devicePixelRatio,
      insets: {
        top: inset("top"),
        right: inset("right"),
        bottom: inset("bottom"),
        left: inset("left"),
      },
    }),
    subscribe: (sync) => {
      // Changing layout during ResizeObserver delivery starts another
      // delivery cycle. Coalesce size changes into the next frame.
      let frame: number | undefined;
      const schedule = (): void => {
        frame ??= window.requestAnimationFrame(() => {
          frame = undefined;
          sync();
        });
      };
      const observer = new ResizeObserver(schedule);
      observer.observe(canvas);
      window.addEventListener("resize", schedule, { passive: true });
      document.addEventListener("fullscreenchange", schedule);
      return () => {
        observer.disconnect();
        if (frame !== undefined) window.cancelAnimationFrame(frame);
        window.removeEventListener("resize", schedule);
        document.removeEventListener("fullscreenchange", schedule);
      };
    },
  };
}

/**
 * Keep CSS layout pixels separate from the WebGPU backing store. The canvas
 * fits the browser viewport in CSS while its drawable size follows device
 * pixels, allowing winit to report native-resolution resize events on HiDPI
 * and fullscreen displays.
 */
export function installCanvasBackingStore(
  canvas: HTMLCanvasElement,
  environment: CanvasEnvironment = browserEnvironment(canvas),
): { readonly sync: () => void; readonly dispose: () => void } {
  let disposed = false;
  const sync = (): void => {
    if (disposed) return;
    const { fullscreen, innerWidth, innerHeight, devicePixelRatio, insets } =
      environment.viewport();
    const safe = fullscreen ? NO_INSETS : (insets ?? NO_INSETS);
    const availableWidth = Math.max(1, innerWidth - (fullscreen ? 0 : 16) - safe.left - safe.right);
    const availableHeight = Math.max(
      1,
      innerHeight - (fullscreen ? 0 : 16) - safe.top - safe.bottom,
    );
    const availableAspect = availableWidth / availableHeight;
    const targetAspect = fullscreen
      ? availableAspect
      : Math.min(16 / 9, Math.max(4 / 3, availableAspect));
    const cssWidth =
      availableAspect >= targetAspect ? availableHeight * targetAspect : availableWidth;
    const cssHeight =
      availableAspect >= targetAspect ? availableHeight : availableWidth / targetAspect;
    canvas.style.width = `${Math.round(cssWidth)}px`;
    canvas.style.height = `${Math.round(cssHeight)}px`;

    const bounds = canvas.getBoundingClientRect();
    const scale = devicePixelRatio || 1;
    const width = Math.max(1, Math.round(bounds.width * scale));
    const height = Math.max(1, Math.round(bounds.height * scale));
    if (canvas.width !== width) canvas.width = width;
    if (canvas.height !== height) canvas.height = height;
  };
  const unsubscribe = environment.subscribe(sync);
  sync();
  return {
    sync,
    dispose: () => {
      if (!disposed) {
        disposed = true;
        unsubscribe();
      }
    },
  };
}
