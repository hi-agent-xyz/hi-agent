import { Component, useEffect, useState, type ComponentType, type ReactNode } from "react";
import { useViews } from "../core/views";

/**
 * Dynamically import a compiled agent view module and render its default export.
 * The module imports `react` / `@hi/core` / `motion/react` as bare specifiers,
 * resolved by the page's import map to the host's shared instances. No props: a
 * view reads the live session through `@hi/core` hooks.
 */
function ViewMount({ moduleUrl }: { moduleUrl: string }) {
  const [Comp, setComp] = useState<ComponentType | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let alive = true;
    setComp(null);
    setFailed(false);
    // The URL is only known at runtime; tell Vite not to try to analyze it.
    import(/* @vite-ignore */ moduleUrl)
      .then((mod) => {
        if (!alive) return;
        setComp(() => mod.default as ComponentType);
      })
      .catch(() => {
        if (alive) setFailed(true);
      });
    return () => {
      alive = false;
    };
  }, [moduleUrl]);

  if (failed || !Comp) return null;
  return <Comp />;
}

/** Contains a render crash in one agent view so it can't take down the host. */
class ViewErrorBoundary extends Component<{ children: ReactNode }, { crashed: boolean }> {
  constructor(props: { children: ReactNode }) {
    super(props);
    this.state = { crashed: false };
  }
  static getDerivedStateFromError() {
    return { crashed: true };
  }
  override render() {
    return this.state.crashed ? null : this.props.children;
  }
}

/**
 * The stage. Each active layer is a bare full-bleed surface keyed by view id —
 * the stable key is the animation-continuity lever (a `replace` under the same id
 * keeps the slot, so a motion-tagged element animates rather than remounting).
 * No default motion: a view appears/leaves instantly unless it opts into motion.
 *
 * **Every view owns the whole frame.** There is no host card, no region and no
 * size class to resolve: the server hands over at most two layers in z-order (the
 * agent's content, then the host's condition notice), and each gets the same
 * `.hi-view-fill` layer with its own background and layout. The frame's only
 * non-negotiable inset is the window chrome and the bottom band the caption dock
 * and controls float in, which `.hi-view-fill` supplies as padding — so a view
 * that lays out nothing of its own still lands legible and clear of the chrome,
 * while a background pinned at `inset: 0` still bleeds edge to edge.
 */
export function ViewSlot() {
  const { views } = useViews();
  if (views.length === 0) return null;
  return (
    <div style={{ position: "fixed", inset: 0, zIndex: 50 }}>
      {views.map((v) => (
        <div key={v.id} className="hi-view-fill">
          <ViewErrorBoundary>
            <ViewMount moduleUrl={v.moduleUrl} />
          </ViewErrorBoundary>
        </div>
      ))}
    </div>
  );
}
