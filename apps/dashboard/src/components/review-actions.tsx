import { useState } from 'react'
import { useMutation, useQueryClient } from '@tanstack/react-query'
import { scrapRun, mergeRun, createPR } from '@/lib/api'
import { Button } from '@/components/ui/button'
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from '@/components/ui/alert-dialog'
import type { Run } from '@/lib/types'

interface ReviewActionsProps {
  run: Run
  onActionComplete?: () => void
}

export function ReviewActions({ run, onActionComplete }: ReviewActionsProps) {
  const queryClient = useQueryClient()
  const [scrapDialogOpen, setScrapDialogOpen] = useState(false)
  const [mergeDialogOpen, setMergeDialogOpen] = useState(false)
  const [resultMessage, setResultMessage] = useState<{ type: 'success' | 'error'; text: string } | null>(null)

  const scrapMutation = useMutation({
    mutationFn: () => scrapRun(run.id),
    onSuccess: () => {
      setResultMessage({ type: 'success', text: 'Branch deleted successfully' })
      queryClient.invalidateQueries({ queryKey: ['run', run.id] })
      queryClient.invalidateQueries({ queryKey: ['runs'] })
      onActionComplete?.()
    },
    onError: (err: Error) => {
      setResultMessage({ type: 'error', text: err.message })
    },
  })

  const mergeMutation = useMutation({
    mutationFn: () => mergeRun(run.id),
    onSuccess: (data) => {
      setResultMessage({ type: 'success', text: `Merged! Commit: ${data.commit.slice(0, 7)}` })
      queryClient.invalidateQueries({ queryKey: ['run', run.id] })
      queryClient.invalidateQueries({ queryKey: ['runs'] })
      onActionComplete?.()
    },
    onError: (err: Error) => {
      setResultMessage({ type: 'error', text: err.message })
    },
  })

  const createPRMutation = useMutation({
    mutationFn: () => createPR(run.id),
    onSuccess: (data) => {
      setResultMessage({ type: 'success', text: `PR created!` })
      queryClient.invalidateQueries({ queryKey: ['run', run.id] })
      // Open PR in new tab
      window.open(data.url, '_blank')
    },
    onError: (err: Error) => {
      setResultMessage({ type: 'error', text: err.message })
    },
  })

  const isLoading = scrapMutation.isPending || mergeMutation.isPending || createPRMutation.isPending
  const branchName = run.worktree?.run_branch
  const targetBranch = run.worktree?.merge_target_branch || run.worktree?.base_branch

  if (!branchName) {
    return null
  }

  return (
    <div className="space-y-3">
      {resultMessage && (
        <div
          className={`px-3 py-2 rounded text-sm ${
            resultMessage.type === 'success'
              ? 'bg-green-100 text-green-800 dark:bg-green-900 dark:text-green-200'
              : 'bg-red-100 text-red-800 dark:bg-red-900 dark:text-red-200'
          }`}
        >
          {resultMessage.text}
        </div>
      )}

      <div className="flex flex-wrap gap-2">
        <Button
          variant="outline"
          size="sm"
          onClick={() => createPRMutation.mutate()}
          disabled={isLoading}
        >
          {createPRMutation.isPending ? 'Creating...' : 'Create PR'}
        </Button>

        <Button
          variant="outline"
          size="sm"
          onClick={() => setMergeDialogOpen(true)}
          disabled={isLoading}
        >
          {mergeMutation.isPending ? 'Merging...' : 'Merge'}
        </Button>

        <Button
          variant="outline"
          size="sm"
          onClick={() => setScrapDialogOpen(true)}
          disabled={isLoading}
          className="text-destructive hover:text-destructive"
        >
          {scrapMutation.isPending ? 'Deleting...' : 'Scrap'}
        </Button>
      </div>

      {/* Scrap Confirmation Dialog */}
      <AlertDialog open={scrapDialogOpen} onOpenChange={setScrapDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Delete branch?</AlertDialogTitle>
            <AlertDialogDescription>
              This will permanently delete the branch <code className="font-mono text-sm bg-muted px-1 rounded">{branchName}</code>.
              All uncommitted changes in the worktree will be lost.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction
              onClick={() => scrapMutation.mutate()}
              className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
            >
              Delete Branch
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      {/* Merge Confirmation Dialog */}
      <AlertDialog open={mergeDialogOpen} onOpenChange={setMergeDialogOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>Merge changes?</AlertDialogTitle>
            <AlertDialogDescription>
              This will merge <code className="font-mono text-sm bg-muted px-1 rounded">{branchName}</code> into{' '}
              <code className="font-mono text-sm bg-muted px-1 rounded">{targetBranch}</code>.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Cancel</AlertDialogCancel>
            <AlertDialogAction onClick={() => mergeMutation.mutate()}>
              Merge
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}
