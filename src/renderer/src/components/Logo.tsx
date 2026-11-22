export function LogoMark({ size = 34 }: { size?: number }) {
  return (
    <svg width={size} height={size} viewBox="0 0 64 64" aria-hidden="true">
      <rect x="2" y="2" width="60" height="60" rx="17" fill="#c53436" />
      <path d="M27 10c2 11-12 17-12 30a17 17 0 0 0 34 0c0-8-5-15-10-20 0 7-3 9-5 10 1-9-2-15-7-20Z" fill="#ffb5ac" />
      <path d="M28 31a2 2 0 0 0-3 1.7v17a2 2 0 0 0 3 1.7l14-8.5a2 2 0 0 0 0-3.4Z" fill="#ffffff" />
    </svg>
  )
}

export function Logo() {
  return (
    <span className="flex items-center gap-2.5">
      <LogoMark />
      <span className="flex flex-col leading-none">
        <span className="font-display text-[21px] font-bold tracking-[-0.01em]">Bara Studio</span>
        <span className="mt-[3px] self-end text-[10.5px] font-medium text-muted">by KrisnaZxx</span>
      </span>
    </span>
  )
}
