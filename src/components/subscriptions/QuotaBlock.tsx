import { useTranslation } from "react-i18next";
import type { SubscriptionQuota } from "../../lib/ipc";
import { formatBytes } from "../../lib/format";
import { cn } from "../../lib/cn";

const DAY_MS = 86_400_000;

function daysLeft(expireUnixSeconds: number): number | null {
  if (expireUnixSeconds <= 0) return null;
  return Math.ceil((expireUnixSeconds * 1000 - Date.now()) / DAY_MS);
}

interface QuotaBlockProps {
  quota: SubscriptionQuota;
  compact?: boolean;
  className?: string;
}

/** Shared subscription usage summary for the Subscriptions and Servers pages. */
export function QuotaBlock({ quota, compact, className }: QuotaBlockProps) {
  const { t } = useTranslation();
  const used = quota.upload + quota.download;
  const unlimited = quota.total <= 0;
  const ratio = unlimited ? 0 : Math.min(1, used / quota.total);
  const days = daysLeft(quota.expire);
  const expired = days !== null && days <= 0;

  return (
    <div className={cn(!compact && "mt-3", className)}>
      {!compact && !unlimited && (
        <div className="h-1 w-full overflow-hidden rounded-full bg-glass-strong">
          <div
            className={cn(
              "h-full rounded-full",
              ratio >= 0.95
                ? "bg-danger"
                : "bg-linear-90 from-accent to-accent-2",
            )}
            style={{ width: `${(ratio * 100).toFixed(1)}%` }}
          />
        </div>
      )}
      <div
        className={cn(
          "flex flex-wrap items-center text-text-faint",
          compact
            ? "gap-x-3 gap-y-0.5 text-[10px]"
            : "gap-x-4 gap-y-1 text-[11px]",
          !compact && !unlimited && "mt-1.5",
        )}
      >
        <span className="tabular-nums">
          {t("subscriptions.trafficUsed", {
            used: formatBytes(used),
            total: unlimited ? "∞" : formatBytes(quota.total),
          })}
        </span>
        {quota.expire > 0 && (
          <span className="tabular-nums">
            {t("subscriptions.expires", {
              date: new Date(quota.expire * 1000).toLocaleDateString(),
            })}
          </span>
        )}
        {days !== null && (
          <span
            className={cn(
              "tabular-nums",
              expired ? "text-danger" : days <= 5 ? "text-warn" : undefined,
            )}
          >
            {expired
              ? t("subscriptions.expired")
              : t("subscriptions.daysLeft", { count: days })}
          </span>
        )}
      </div>
    </div>
  );
}
