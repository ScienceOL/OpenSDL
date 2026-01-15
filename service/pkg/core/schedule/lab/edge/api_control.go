package edge

import (
	// 外部依赖
	"context"
	"encoding/json"
	"reflect"

	// 内部引用
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	action "github.com/scienceol/opensdl/service/pkg/core/schedule/engine/action"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

// 处理控制类消息

// control 消息
func (e *EdgeImpl) onControlMessage(ctx context.Context, msg string) {
	logger.Infof(ctx, "schedule msg OnJobMessage job msg: %s", msg)
	apiType := &lab.ApiControlMsg{}
	if err := json.Unmarshal([]byte(msg), apiType); err != nil {
		logger.Errorf(ctx, "onControlMessage err: %+v, msg: %s", err, msg)
		return
	}

	switch apiType.Action {
	case lab.StartAction:
		e.onStartAction(ctx, msg)
	case lab.StopJob:
		e.onStopJob(ctx, msg)
	case lab.StatusJob:
		e.onStatusJob(ctx, msg)
	case lab.AddMaterial, lab.UpdateMaterial, lab.RemoveMaterial:
		e.onMaterial(ctx, msg)
	default:
		logger.Errorf(ctx, "EdgeImpl.onControlMessage unknown action: %s", apiType.Action)
	}
}

func (e *EdgeImpl) onStartAction(ctx context.Context, msg string) {
	apiMsg := &lab.ApiControlData[engine.WorkflowInfo]{}
	if err := json.Unmarshal([]byte(msg), apiMsg); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onWorkflowJob unmarshal err: %+v", err)
		return
	}

	defer func() { e.actionTask = nil }()
	e.actionTask = action.NewActionTask(ctx, &engine.ActionParam{
		Session:      e.labInfo.Session,
		Sandbox:      e.labInfo.Sandbox,
		WorkflowInfo: &apiMsg.Data,
	})

	if err := utils.SafelyRun(func() {
		e.actionTask.Run()
	}); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onNotebookJob err: %+v", err)
	}
}

func (e *EdgeImpl) onStopJob(ctx context.Context, msg string) {
	// 停止 workflow 、notebook
	if e.jobTask == nil {
		return
	} else {
		v := reflect.ValueOf(e.jobTask)
		if v.Kind() == reflect.Ptr && v.IsNil() {
			return
		}
	}

	apiControlData := &lab.ApiControlData[lab.StopJobReq]{}
	if err := json.Unmarshal([]byte(msg), apiControlData); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onAddMaterial unmarshal err: %+v", err)
		return
	}

	if apiControlData.Data.UUID == e.jobTask.ID(ctx) {
		e.jobTask.Stop()
	}
}

func (e *EdgeImpl) onStatusJob(ctx context.Context, msg string) {
	panic("not implements")
}

func (e *EdgeImpl) onMaterial(ctx context.Context, msg string) {
	apiControlData := &lab.ApiControlData[any]{}
	if err := json.Unmarshal([]byte(msg), apiControlData); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onAddMaterial unmarshal err: %+v", err)
		return
	}

	data := map[string]any{
		"action":       apiControlData.Action,
		"data":         apiControlData.Data,
		"edge_session": apiControlData.Session,
	}

	dataB, _ := json.Marshal(data)
	if err := e.labInfo.Session.Write(dataB); err != nil {
		logger.Errorf(ctx, "EdgeImpl.onAddMaterial notifyAddMaterial data: %s, err: %+v", string(dataB), err)
	}
}
