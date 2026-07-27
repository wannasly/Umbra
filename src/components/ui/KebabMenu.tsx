import { useRef, useState, type ReactNode } from "react";
import { EllipsisVertical } from "lucide-react";
import { cn } from "../../lib/cn";
import { IconButton } from "./IconButton";
import { Popover } from "./Popover";

export interface KebabMenuItem {
  label: string;
  icon?: ReactNode;
  danger?: boolean;
  onClick: () => void;
}

export function KebabMenu({ items }: { items: KebabMenuItem[] }) {
  const [open, setOpen] = useState(false);
  const anchorRef = useRef<HTMLDivElement>(null);

  return (
    <div
      ref={anchorRef}
      className="relative"
      onClick={(e) => e.stopPropagation()}
    >
      <IconButton onClick={() => setOpen((o) => !o)}>
        <EllipsisVertical size={15} />
      </IconButton>
      <Popover
        open={open}
        onClose={() => setOpen(false)}
        anchorRef={anchorRef}
        className="min-w-44"
      >
        {items.map((item) => (
          <button
            key={item.label}
            type="button"
            onClick={(e) => {
              e.stopPropagation();
              setOpen(false);
              item.onClick();
            }}
            className={cn(
              "flex w-full items-center gap-2.5 px-3 py-2 text-left text-[13px] whitespace-nowrap",
              "transition-colors duration-100",
              item.danger
                ? "text-danger hover:bg-danger/10"
                : "text-text-dim hover:bg-glass-strong hover:text-text",
            )}
          >
            {item.icon}
            {item.label}
          </button>
        ))}
      </Popover>
    </div>
  );
}
