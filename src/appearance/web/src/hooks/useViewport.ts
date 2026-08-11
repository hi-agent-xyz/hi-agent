import { useEffect, useState } from "react";

/**
 * The window's width, for the one decision that needs it: whether there is room
 * for the conversation rail beside a view (`RAIL_MIN_WIDTH`).
 *
 * A resize listener rather than a media query because the threshold belongs to
 * the compositor — the pass that decides the rail is pure and testable, and it
 * cannot be if half its input lives in a stylesheet.
 */
export function useViewportWidth(): number {
  const [width, setWidth] = useState(() =>
    typeof window === "undefined" ? 1440 : window.innerWidth,
  );
  useEffect(() => {
    const onResize = () => setWidth(window.innerWidth);
    window.addEventListener("resize", onResize);
    return () => window.removeEventListener("resize", onResize);
  }, []);
  return width;
}
