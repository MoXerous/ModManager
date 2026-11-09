interface SegmentedProps<T extends string> {
  label: string;
  value: T;
  options: readonly { value: T; label: string }[];
  onChange: (value: T) => void;
}

/**
 * A small radio group for a closed set of choices.
 *
 * Extracted from Settings so Browse can offer the same control for the
 * provider without a second implementation drifting from it.
 */
export function Segmented<T extends string>({
  label,
  value,
  options,
  onChange,
}: SegmentedProps<T>) {
  return (
    <div className="flex items-center justify-between gap-2 text-small text-muted-foreground">
      <span>{label}</span>
      <div
        role="radiogroup"
        aria-label={label}
        className="flex rounded-segmented border border-border bg-surface-sunken p-0.5"
      >
        {options.map((option) => (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={option.value === value}
            onClick={() => onChange(option.value)}
            className={
              option.value === value
                ? 'rounded-control bg-accent px-[9px] py-[3px] text-foreground'
                : 'rounded-control px-[9px] py-[3px] text-muted-foreground'
            }
          >
            {option.label}
          </button>
        ))}
      </div>
    </div>
  );
}
