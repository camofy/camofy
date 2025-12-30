type LoginPanelProps = {
  loginPassword: string
  onLoginPasswordChange: (value: string) => void
  onSubmit: (event: React.FormEvent<HTMLFormElement>) => void
  loading: boolean
}

function LoginPanel({
  loginPassword,
  onLoginPasswordChange,
  onSubmit,
  loading,
}: LoginPanelProps) {
  return (
    <main className="flex flex-1 items-center justify-center">
      <form
        onSubmit={onSubmit}
        className="w-full max-w-sm rounded-lg border border-[color:var(--color-border-strong)] bg-[color:var(--color-surface)] p-4 shadow-lg"
      >
        <h2 className="text-base font-medium text-[color:var(--color-text-main)]">登录面板</h2>
        <p className="mt-1 text-xs text-[color:var(--color-text-muted)]">
          该面板已设置访问密码，请输入密码以继续。
        </p>
        <div className="mt-3">
          <label className="block text-xs font-medium text-[color:var(--color-text-main)]">面板密码</label>
          <input
            type="password"
            className="mt-1 w-full rounded border border-[color:var(--color-border-subtle)] bg-[color:var(--color-surface-soft)] px-3 py-1.5 text-sm text-[color:var(--color-text-main)] outline-none ring-0 focus:border-[color:var(--color-primary)]"
            value={loginPassword}
            onChange={(e) => onLoginPasswordChange(e.target.value)}
            autoComplete="current-password"
          />
        </div>
        <button
          type="submit"
          disabled={loading}
          className="mt-4 w-full rounded bg-[color:var(--color-primary)] px-3 py-1.5 text-sm font-medium text-[color:var(--color-primary-on)] hover:bg-[#6b6f63] disabled:cursor-not-allowed disabled:opacity-60"
        >
          {loading ? '正在登录…' : '登录'}
        </button>
      </form>
    </main>
  )
}

export default LoginPanel
