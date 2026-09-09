// `d3-flextree` publishes no types and there is no `@types/d3-flextree`, so the shape it
// actually has is declared here rather than letting the shim beside it resolve to `any`.
//
// Only what a caller needs: the layout is a callable that also carries `hierarchy`, and it
// reads a node's size through the accessor rather than from the datum, which is the whole
// difference from `d3-hierarchy`'s `tree()`.
declare module "d3-flextree" {
  export interface FlextreeNode<T> {
    data: T;
    /** Breadth — across the siblings. The tree is laid out along this axis. */
    x: number;
    /** Depth — one step per rank. */
    y: number;
    parent: FlextreeNode<T> | null;
    children?: FlextreeNode<T>[];
    descendants(): FlextreeNode<T>[];
    each(fn: (node: FlextreeNode<T>) => void): FlextreeNode<T>;
  }

  export interface FlextreeLayout<T> {
    (root: FlextreeNode<T>): FlextreeNode<T>;
    hierarchy(data: T, children?: (datum: T) => T[] | null | undefined): FlextreeNode<T>;
    nodeSize(): (node: FlextreeNode<T>) => [number, number];
    nodeSize(accessor: (node: FlextreeNode<T>) => [number, number]): FlextreeLayout<T>;
    spacing(): (a: FlextreeNode<T>, b: FlextreeNode<T>) => number;
    spacing(value: number | ((a: FlextreeNode<T>, b: FlextreeNode<T>) => number)): FlextreeLayout<T>;
  }

  export function flextree<T = unknown>(options?: {
    children?: (datum: T) => T[] | null | undefined;
    nodeSize?: (node: FlextreeNode<T>) => [number, number];
    spacing?: number | ((a: FlextreeNode<T>, b: FlextreeNode<T>) => number);
  }): FlextreeLayout<T>;
}
