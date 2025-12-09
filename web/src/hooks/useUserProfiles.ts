import { useCallback, useState } from 'react'
import type {
  UserProfileDetail,
  UserProfileSummary,
} from '../types'
import { useAuth } from '../context/AuthContext'
import { useNotifications } from '../context/NotificationContext'
import {
  activateUserProfile,
  createUserProfile,
  deleteUserProfile,
  getUserProfileDetail,
  listUserProfiles,
  updateUserProfile,
} from '../api'

export function useUserProfiles() {
  const { authedFetch } = useAuth()
  const { notifyError, notifySuccess } = useNotifications()

  const [profiles, setProfiles] = useState<UserProfileSummary[]>([])
  const [activeId, setActiveId] = useState<string | null>(null)
  const [loading, setLoading] = useState(false)
  const [saving, setSaving] = useState(false)
  const [creating, setCreating] = useState(false)

  const load = useCallback(async () => {
    setLoading(true)
    try {
      const list = await listUserProfiles(authedFetch)
      setProfiles(list)
      const active = list.find((p) => p.is_active)
      setActiveId(active ? active.id : null)
    } catch (err) {
      const msg =
        err instanceof Error ? err.message : '加载用户配置列表失败'
      notifyError(msg)
    } finally {
      setLoading(false)
    }
  }, [authedFetch, notifyError])

  const loadDetail = useCallback(
    async (id: string): Promise<UserProfileDetail | null> => {
      try {
        const detail = await getUserProfileDetail(authedFetch, id)
        return detail
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '加载用户配置失败'
        notifyError(msg)
        return null
      }
    },
    [authedFetch, notifyError],
  )

  const create = useCallback(
    async (name: string, content: string): Promise<UserProfileSummary | null> => {
      const trimmed = name.trim()
      if (!trimmed) {
        notifyError('用户 profile 名称不能为空')
        return null
      }
      setCreating(true)
      try {
        const summary = await createUserProfile(authedFetch, {
          name: trimmed,
          content,
        })
        notifySuccess('用户配置已创建')
        await load()
        return summary
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '创建用户配置失败'
        notifyError(msg)
        return null
      } finally {
        setCreating(false)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  const update = useCallback(
    async (params: {
      id: string
      name: string
      content: string
    }): Promise<UserProfileDetail | null> => {
      const trimmed = params.name.trim()
      if (!trimmed) {
        notifyError('用户 profile 名称不能为空')
        return null
      }
      setSaving(true)
      try {
        const detail = await updateUserProfile(authedFetch, {
          id: params.id,
          name: trimmed,
          content: params.content,
        })
        notifySuccess('用户配置已保存并合并')
        await load()
        return detail
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '保存用户配置失败'
        notifyError(msg)
        return null
      } finally {
        setSaving(false)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  const remove = useCallback(
    async (id: string) => {
      try {
        await deleteUserProfile(authedFetch, id)
        notifySuccess('用户配置已删除')
        await load()
        if (activeId === id) {
          setActiveId(null)
        }
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '删除用户配置失败'
        notifyError(msg)
      }
    },
    [activeId, authedFetch, load, notifyError, notifySuccess],
  )

  const activate = useCallback(
    async (id: string) => {
      try {
        await activateUserProfile(authedFetch, id)
        notifySuccess('已设置当前用户配置')
        setActiveId(id)
        await load()
      } catch (err) {
        const msg =
          err instanceof Error ? err.message : '设置活跃用户配置失败'
        notifyError(msg)
      }
    },
    [authedFetch, load, notifyError, notifySuccess],
  )

  return {
    profiles,
    activeId,
    loading,
    saving,
    creating,
    load,
    loadDetail,
    create,
    update,
    remove,
    activate,
  }
}

