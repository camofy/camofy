import type { ReactNode } from 'react'

type AnsiStyle = {
  foreground?: AnsiColor
  background?: AnsiColor
  bold?: boolean
}

type AnsiColor =
  | 'black'
  | 'red'
  | 'green'
  | 'yellow'
  | 'blue'
  | 'magenta'
  | 'cyan'
  | 'white'
  | 'bright-black'
  | 'bright-red'
  | 'bright-green'
  | 'bright-yellow'
  | 'bright-blue'
  | 'bright-magenta'
  | 'bright-cyan'
  | 'bright-white'

type Segment = {
  text: string
  className: string
}

// eslint-disable-next-line no-control-regex
const ANSI_REGEX = /\x1b\[[0-9;]*m/g

const FG_COLOR_MAP: Record<number, AnsiColor> = {
  30: 'black',
  31: 'red',
  32: 'green',
  33: 'yellow',
  34: 'blue',
  35: 'magenta',
  36: 'cyan',
  37: 'white',
  90: 'bright-black',
  91: 'bright-red',
  92: 'bright-green',
  93: 'bright-yellow',
  94: 'bright-blue',
  95: 'bright-magenta',
  96: 'bright-cyan',
  97: 'bright-white',
}

const BG_COLOR_MAP: Record<number, AnsiColor> = {
  40: 'black',
  41: 'red',
  42: 'green',
  43: 'yellow',
  44: 'blue',
  45: 'magenta',
  46: 'cyan',
  47: 'white',
  100: 'bright-black',
  101: 'bright-red',
  102: 'bright-green',
  103: 'bright-yellow',
  104: 'bright-blue',
  105: 'bright-magenta',
  106: 'bright-cyan',
  107: 'bright-white',
}

const styleToClassName = (style: AnsiStyle): string => {
  const classes: string[] = []
  if (style.bold) {
    classes.push('ansi-bold')
  }
  if (style.foreground) {
    classes.push(`ansi-fg-${style.foreground}`)
  }
  if (style.background) {
    classes.push(`ansi-bg-${style.background}`)
  }
  return classes.join(' ')
}

const applySgrCodes = (codes: number[], style: AnsiStyle): AnsiStyle => {
  if (codes.length === 0) {
    // ESC[m 等价于重置
    codes = [0]
  }

  for (const code of codes) {
    if (code === 0) {
      style = {}
    } else if (code === 1) {
      style = { ...style, bold: true }
    } else if (code === 22) {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { bold, ...rest } = style
      style = rest
    } else if (code >= 30 && code <= 37) {
      const color = FG_COLOR_MAP[code]
      style = color ? { ...style, foreground: color } : style
    } else if (code === 39) {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { foreground, ...rest } = style
      style = rest
    } else if ((code >= 40 && code <= 47) || (code >= 100 && code <= 107)) {
      const color = BG_COLOR_MAP[code]
      style = color ? { ...style, background: color } : style
    } else if (code === 49) {
      // eslint-disable-next-line @typescript-eslint/no-unused-vars
      const { background, ...rest } = style
      style = rest
    } else {
      // 其他 SGR 码暂不处理（例如下划线、斜体等）
    }
  }

  return style
}

const parseAnsi = (text: string): Segment[] => {
  const segments: Segment[] = []
  let currentStyle: AnsiStyle = {}
  let currentClass = styleToClassName(currentStyle)
  let buffer = ''
  let lastIndex = 0

  const regex = new RegExp(ANSI_REGEX, 'g')

   
  while (true) {
    const match = regex.exec(text)
    if (!match) {
      break
    }

    const chunk = text.slice(lastIndex, match.index)
    if (chunk) {
      buffer += chunk
    }

    if (buffer) {
      segments.push({ text: buffer, className: currentClass })
      buffer = ''
    }

    const sequence = match[0]
    const content = sequence.slice(2, -1) // 去掉 \x1b[ 和 m
    const codes =
      content.trim().length === 0
        ? [0]
        : content
            .split(';')
            .map((c) => Number.parseInt(c, 10))
            .filter((n) => !Number.isNaN(n))

    currentStyle = applySgrCodes(codes, currentStyle)
    currentClass = styleToClassName(currentStyle)
    lastIndex = regex.lastIndex
  }

  const rest = text.slice(lastIndex)
  if (rest) {
    buffer += rest
  }
  if (buffer) {
    segments.push({ text: buffer, className: currentClass })
  }

  return segments
}

type AnsiTextProps = {
  text: string
}

function AnsiText({ text }: AnsiTextProps): ReactNode {
  const segments = parseAnsi(text)

  // 没有任何 ANSI 控制码，直接返回原始字符串
  if (segments.length === 1 && segments[0].className === '') {
    return segments[0].text
  }

  return segments.map((segment, index) =>
    segment.className ? (
      <span key={index} className={segment.className}>
        {segment.text}
      </span>
    ) : (
      <span key={index}>{segment.text}</span>
    ),
  )
}

export default AnsiText

