/** Thumbnail with a fixed max height. A neutral backdrop keeps transparent images readable. */
export function ImagePreview({
  url,
  width,
  height,
  maxHeight,
}: {
  url: string | null;
  width: number;
  height: number;
  maxHeight: number;
}) {
  if (!url) {
    return <span className="text-sm text-neutral-400">Preview unavailable</span>;
  }
  return (
    <img
      src={url}
      alt={`Copied image, ${width}×${height}`}
      // Reserve the right box before the image decodes so rows don't jump.
      style={{ aspectRatio: `${width} / ${height}`, maxHeight }}
      className="max-w-full rounded-sm bg-neutral-100 object-contain outline outline-1 -outline-offset-1 outline-black/10 dark:bg-neutral-800 dark:outline-white/10"
    />
  );
}
