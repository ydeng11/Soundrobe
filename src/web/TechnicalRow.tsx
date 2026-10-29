/**
 * One labelled technical fact: a muted label, its value on the right.
 *
 * Three views render this markup — the monitoring schedule, the settings runtime
 * block and the track detail editor — and it had been copied into each of them
 * under two different names, so the copies could drift apart unnoticed.
 */

export function TechnicalRow({
  label,
  value,
  mono,
}: {
  label: string;
  value: string;
  /** A machine-readable value — a crontab, an identifier — shown in the log face. */
  mono?: boolean;
}) {
  return (
    <span className="sr-technical__row">
      <span className="sr-technical__label">{label}</span>
      <span className={mono ? "sr-log-inline" : undefined}>{value}</span>
    </span>
  );
}
