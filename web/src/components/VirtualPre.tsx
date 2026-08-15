import { useEffect, useMemo, useRef, useState } from 'react'

type VirtualPreProps = {
  text: string
  className?: string
  lineHeight?: number
}

const OVERSCAN = 12

function VirtualPre({ text, className, lineHeight = 22 }: VirtualPreProps) {
  const scrollerRef = useRef<HTMLDivElement>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewport, setViewport] = useState(320)

  const lines = useMemo(() => text.split('\n'), [text])

  useEffect(() => {
    const el = scrollerRef.current
    if (!el) return

    const update = () => setViewport(el.clientHeight)
    update()

    const observer = new ResizeObserver(update)
    observer.observe(el)
    return () => observer.disconnect()
  }, [])

  const visibleCount = Math.ceil(viewport / lineHeight) + OVERSCAN * 2
  const start = Math.max(0, Math.floor(scrollTop / lineHeight) - OVERSCAN)
  const end = Math.min(lines.length, start + visibleCount)
  const offsetY = start * lineHeight

  return (
    <div
      ref={scrollerRef}
      className="h-full min-h-0 overflow-auto"
      onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}
    >
      <div style={{ height: lines.length * lineHeight, position: 'relative' }}>
        <pre
          className={className}
          style={{
            position: 'absolute',
            top: offsetY,
            left: 0,
            right: 0,
            margin: 0,
            whiteSpace: 'pre',
          }}
        >
          {lines.slice(start, end).join('\n')}
        </pre>
      </div>
    </div>
  )
}

export default VirtualPre
