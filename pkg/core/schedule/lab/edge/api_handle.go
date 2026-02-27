package edge

import (
	"context"
	"encoding/json"

	"github.com/scienceol/osdl/pkg/core/schedule/engine"
	"github.com/scienceol/osdl/pkg/core/schedule/engine/dag"
	en "github.com/scienceol/osdl/pkg/core/schedule/engine/notebook"
	"github.com/scienceol/osdl/pkg/core/schedule/lab"
	"github.com/scienceol/osdl/pkg/middleware/logger"
	"github.com/scienceol/osdl/pkg/utils"
)

//	处理 api 任务类消息

// job 运行工作流消息
func (e *EdgeImpl) onJobMessage(ctx context.Context, msg string) {
	logger.Infof(ctx, "schedule msg OnJobMessage job msg: %s", msg)
	apiType := &lab.ApiMsg{}
	if err := json.Unmarshal([]byte(msg), apiType); err != nil {
		logger.Errorf(ctx, "OnJobMessage err: %+v, msg: %s", err, msg)
		return
	}

	switch apiType.Action {
	case lab.StartWorkflow:
		e.onWorkflowJob(ctx, msg)
	case lab.StartNotebook:
		e.onNotebookJob(ctx, msg)
	default:
		logger.Errorf(ctx, "EdgeImpl.onJobMessage unknown action: %s", apiType.Action)
	}
}

func (e *EdgeImpl) onWorkflowJob(ctx context.Context, msg string) {
	apiMsg := &lab.ApiData[engine.WorkflowInfo]{}
	if err := json.Unmarshal([]byte(msg), apiMsg); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onWorkflowJob unmarshal err: %+v", err)
		return
	}

	defer func() { e.jobTask = nil }()

	e.jobTask = dag.NewDagTask(ctx, &engine.TaskParam{
		Session: e.labInfo.Session,
		Sandbox: e.labInfo.Sandbox,
		WorkflowInfo: &engine.WorkflowInfo{
			TaskUUID:     apiMsg.Data.TaskUUID,
			WorkflowUUID: apiMsg.Data.WorkflowUUID,
			LabUUID:      e.labInfo.UUID,
			LabID:        e.labInfo.ID,
			UserID:       e.labInfo.LabUserID,
		},
	})

	defer e.jobTask.Stop()

	if err := utils.SafelyRun(func() {
		e.jobTask.Run()
	}); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onWorkflowJob err: %+v", err)
	}
}

func (e *EdgeImpl) onNotebookJob(ctx context.Context, msg string) {
	apiMsg := &lab.ApiData[engine.NotebookInfo]{}
	if err := json.Unmarshal([]byte(msg), apiMsg); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onWorkflowJob unmarshal err: %+v", err)
		return
	}

	defer func() { e.jobTask = nil }()
	e.jobTask = en.NewNotebookTask(ctx, &engine.NotebookInfo{
		Session:      e.labInfo.Session,
		Sandbox:      e.labInfo.Sandbox,
		NotebookUUID: apiMsg.Data.NotebookUUID,
		LabUUID:      e.labInfo.UUID,
		LabID:        e.labInfo.ID,
		UserID:       e.labInfo.LabUserID,
	})

	defer e.jobTask.Stop()

	if err := utils.SafelyRun(func() {
		e.jobTask.Run()
	}); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onNotebookJob err: %+v", err)
	}
}
