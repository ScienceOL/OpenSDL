package mcp

import (
	// 外部依赖
	"context"
	"encoding/json"
	"sort"
	"time"

	r "github.com/redis/go-redis/v9"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	uuid "github.com/scienceol/opensdl/service/pkg/common/uuid"
	mcp "github.com/scienceol/opensdl/service/pkg/core/mcp"
	engine "github.com/scienceol/opensdl/service/pkg/core/schedule/engine"
	action "github.com/scienceol/opensdl/service/pkg/core/schedule/engine/action"
	lab "github.com/scienceol/opensdl/service/pkg/core/schedule/lab"
	auth "github.com/scienceol/opensdl/service/pkg/middleware/auth"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	redis "github.com/scienceol/opensdl/service/pkg/middleware/redis"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
	model "github.com/scienceol/opensdl/service/pkg/model"
	wfl "github.com/scienceol/opensdl/service/pkg/repo/workflow"
	utils "github.com/scienceol/opensdl/service/pkg/utils"
)

type mcpImpl struct {
	rClient       *r.Client
	workflowStore repo.WorkflowRepo
}

func New() mcp.Service {
	return &mcpImpl{
		rClient:       redis.GetClient(),
		workflowStore: wfl.New(),
	}
}

func (m *mcpImpl) RunAction(ctx context.Context, req *action.RunActionReq) (*action.RunActionResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	req.UUID = uuid.NewV4()
	logger.Infof(ctx, "====================== %s", req.UUID)
	if exists, err := m.rClient.Exists(ctx, utils.LabHeartName(req.LabUUID)).Result(); err != nil ||
		exists == 0 {
		return nil, code.EdgeNotStartedErr
	}

	data := lab.ApiControlData[engine.WorkflowInfo]{
		ApiControlMsg: lab.ApiControlMsg{
			Action: lab.StartAction,
		},
		Data: engine.WorkflowInfo{
			TaskUUID:     req.UUID,
			WorkflowUUID: req.UUID,
			LabUUID:      req.LabUUID,
			UserID:       userInfo.ID,
		},
	}

	conf := config.Global().Job
	dataB, _ := json.Marshal(data)
	logger.Infof(ctx, "RunAction data: %+v", data)
	paramB, _ := json.Marshal(req)

	setRet := m.rClient.SetEx(ctx, action.ActionKey(req.UUID), paramB, 24*time.Hour)
	if setRet.Err() != nil {
		logger.Errorf(ctx, "RunAction send param error: %+v", setRet.Err())
		return nil, code.ParamErr.WithMsgf("set RunAction param err: %+v", setRet.Err())
	}

	logger.Infof(ctx, "================================ queue %s", conf.JobQueueName)
	ret := m.rClient.LPush(ctx, utils.LabControlName(req.LabUUID), dataB)
	if ret.Err() != nil {
		logger.Errorf(ctx, "RunAction send data error: %+v", ret.Err())
		return nil, code.ParamErr.WithMsgf("push workflow redis msg err: %+v", ret.Err())
	}

	resp := &action.RunActionResp{}
	count := 120
	for count > 0 {
		count--
		time.Sleep(time.Second)
		getRet := m.rClient.Get(ctx, action.ActionRetKey(req.UUID))

		err := getRet.Err()
		if err != nil && err.Error() != "redis: nil" {
			logger.Errorf(ctx, "RunAction send param error: %+v", getRet.Err())
			return nil, code.ParamErr.WithMsgf("set RunAction param err: %+v", setRet.Err())
		} else if err != nil && err.Error() == "redis: nil" {
			continue
		}

		err = json.Unmarshal([]byte(getRet.Val()), resp)
		if err != nil {
			return nil, code.ParamErr.WithErr(err)
		}

		if resp.Status == "failed" || resp.Status == "success" {
			break
		}
	}

	return resp, nil
}

func (m *mcpImpl) QueryTaskStatus(ctx context.Context, req *mcp.TaskStatusReq) (*mcp.TaskStatusResp, error) {
	userInfo := auth.GetCurrentUser(ctx)
	if userInfo == nil {
		return nil, code.UnLogin
	}

	task := &model.WorkflowTask{}
	if err := m.workflowStore.GetData(ctx, task, map[string]any{
		"uuid": req.UUID,
	}); err != nil {
		return nil, err
	}

	jobs := make([]*model.WorkflowNodeJob, 0, 10)
	if err := m.workflowStore.FindDatas(ctx, &jobs, map[string]any{
		"workflow_task_id": task.ID,
	}); err != nil {
		return nil, err
	}

	nodeIDs := utils.FilterUniqSlice(jobs, func(j *model.WorkflowNodeJob) (int64, bool) {
		return j.NodeID, true
	})

	nodes := make([]*model.WorkflowNode, 0, 10)
	if err := m.workflowStore.FindDatas(ctx, &nodes, map[string]any{
		"id": nodeIDs,
	}, "id", "name", "action_name"); err != nil {
		return nil, err
	}

	nodeMap := utils.Slice2Map(nodes, func(n *model.WorkflowNode) (int64, *model.WorkflowNode) {
		return n.ID, n
	})

	sort.Slice(jobs, func(i, j int) bool {
		return jobs[i].ID < jobs[j].ID
	})

	return &mcp.TaskStatusResp{
		Status: string(task.Status),
		JosStatus: utils.FilterSlice(jobs, func(j *model.WorkflowNodeJob) (*mcp.TaskJobStatus, bool) {
			nodeName := ""
			actionName := ""
			node, ok := nodeMap[j.NodeID]
			if ok {
				nodeName = node.Name
				actionName = node.ActionName
			}
			return &mcp.TaskJobStatus{
				UUID:       j.UUID,
				NodeName:   nodeName,
				ActionName: actionName,
				Status:     string(j.Status),
				ReturnInfo: j.ReturnInfo,
			}, true
		}),
	}, nil
}
