export type PlanStatus = 'pending' | 'in_progress' | 'completed'

export interface PlanItem {
  id: string
  text: string
  status: PlanStatus
}

export interface PlanSummary {
  completed: number
  total: number
  all_completed: boolean
}

export interface PlanSnapshot {
  action: 'set' | 'update' | 'get' | 'clear'
  workspace: string
  changed: boolean
  items: PlanItem[]
  summary: PlanSummary
}
