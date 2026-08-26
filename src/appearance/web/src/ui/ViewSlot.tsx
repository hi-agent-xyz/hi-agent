import { url } from "../lib/base";
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
    // It comes from the backend as a root-absolute path, which is the core's
    // root — not necessarily this page's, when the community serves the core
    // under a subpath.
    import(/* @vite-ignore */ url(moduleUrl))
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
 * **Every view owns the frame it is handed.** There is no host card, no region and
 * no size class to resolve: the server hands over at most two layers in z-order
 * (the agent's content, then the host's condition notice), and each gets the same
 * `.hi-view-fill` layer with its own layout. **The frame is the window and nothing
 * is subtracted from it** — not for the titlebar, not for the controls, and not for
 * the conversation, which is a popover over this layer rather than a rail beside it.
 * Everything the host floats carries its own ground instead (the cluster and the
 * pills wear a scrim), so a view's content reaches every edge exactly like its
 * ground does, and a picture pinned at `inset: 0` fills the window. What a view
 * owes the chrome is placement, not space: `--hi-safe-top` and `--hi-chrome-bottom`
 * are published for the views that want to be exact about it.
 *
 * **The frame is fixed, and the layer scrolls.** A view taller than the frame used
 * to be cut at the window edge with nothing to reach the rest — `.hi-root` is
 * `overflow: hidden` and this layer declared none. It now carries `overflow-y: auto`,
 * so the whole of a view is reachable whatever the window is doing. That is a floor,
 * not a licence: the person is being talked through the view and may never scroll it,
 * so what matters still belongs on the first screen.
 *
 * The slot is the `view` plane's whole occupant and carries no `z-index` of its
 * own: paint order inside a plane is DOM order, which here is already the wire's
 * z-order. A view writing `z-index: 9999` therefore climbs to the top of this
 * plane and no further — it can never reach the conversation or the controls.
 * See `docs/arch/stage.md`.
 *
 * The layer also carries the ground (`--paper`, the presence's own), so a themed
 * view paints no background at all and one that ends short still stands on paper.
 * A view with a palette that isn't the theme's brings its own, and the border-not-
 * padding inset is what lets that one melt into the window instead of sitting in a
 * frame of paper the view never drew.
 */
export function ViewSlot() {
  const { views } = useViews();
  if (views.length === 0) return null;
  return (
    <>
      {views.map((v) => (
        <div key={v.id} className="hi-view-fill">
          <ViewErrorBoundary>
            <ViewMount moduleUrl={v.moduleUrl} />
          </ViewErrorBoundary>
        </div>
      ))}
    </>
  );
}
