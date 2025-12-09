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
        className="w-full max-w-sm rounded-lg border border-slate-800 bg-slate-900/80 p-4 shadow-lg"
      >
        <h2 className="text-base font-medium text-slate-100">登录面板</h2>
        <p className="mt-1 text-xs text-slate-400">
          该面板已设置访问密码，请输入密码以继续。
        </p>
        <div className="mt-3">
          <label className="block text-xs font-medium text-slate-300">面板密码</label>
          <input
            type="password"
            className="mt-1 w-full rounded border border-slate-700 bg-slate-900 px-3 py-1.5 text-sm text-slate-100 outline-none ring-0 focus:border-sky-500"
            value={loginPassword}
            onChange={(e) => onLoginPasswordChange(e.target.value)}
            autoComplete="current-password"
          />
        </div>
        <button
          type="submit"
          disabled={loading}
          className="mt-4 w-full rounded bg-sky-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-sky-500 disabled:cursor-not-allowed disabled:opacity-60"
        >
          {loading ? '正在登录…' : '登录'}
        </button>
      </form>
    </main>
  )
}

export default LoginPanel
