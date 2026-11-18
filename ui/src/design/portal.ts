// Moves an element into a region of the window by id, so popovers and side
// panels sit where the layout puts them (decisions/09-ui.md §2) while their
// component stays where its state lives. Without the region (component
// tests) the element stays in place.
export function portal(id: string) {
  return (node: HTMLElement) => {
    const target = document.getElementById(id);
    if (!target) return;
    target.append(node);
    return () => node.remove();
  };
}
