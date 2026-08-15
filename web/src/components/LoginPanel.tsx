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
    <form onSubmit={onSubmit} className="ui-card w-full max-w-md p-10">
      <div className="mb-8 flex items-center gap-4">
        <span className="brand-mark h-12 w-12 rounded-2xl">C</span>
        <h2 className="text-2xl font-semibold">登录</h2>
      </div>
      <label className="text-lg text-[color:var(--color-text-muted)] block">密码</label>
      <input
        type="password"
        className="ui-input mt-2"
        value={loginPassword}
        onChange={(e) => onLoginPasswordChange(e.target.value)}
        autoComplete="current-password"
      />
      <button type="submit" disabled={loading} className="ui-btn-primary mt-8 w-full">
        {loading ? '登录中…' : '登录'}
      </button>
    </form>
  )
}

export default LoginPanel
