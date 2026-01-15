package machineimpl

import (
	// 外部依赖
	"context"
	"net/http"
	"strconv"
	"strings"
	"time"

	"github.com/go-resty/resty/v2"

	// 内部引用
	config "github.com/scienceol/opensdl/service/internal/config"
	common "github.com/scienceol/opensdl/service/pkg/common"
	code "github.com/scienceol/opensdl/service/pkg/common/code"
	logger "github.com/scienceol/opensdl/service/pkg/middleware/logger"
	model "github.com/scienceol/opensdl/service/pkg/model"
	repo "github.com/scienceol/opensdl/service/pkg/repo"
)

const (
	UserIdHeaderKey = "X-User-Id"
	OrgIdHeaderKey  = "X-Org-Id"
)

type CreateNodeResp struct {
	ID int64
}

type MachineImpl struct {
	*resty.Client
}

func NewMachine() repo.Machine {
	conf := config.Global().RPC.BohrCore
	return &MachineImpl{
		Client: resty.New().
			EnableTrace().
			SetTimeout(5 * time.Second).
			SetBaseURL(conf.Addr),
	}
}

func (m *MachineImpl) DelMachine(ctx context.Context, req *model.DelMachineReq) error {
	createID, _ := strconv.ParseUint(req.UserID, 10, 64)
	req.CreatorID = createID
	req.Device = "container"
	res := common.Resp{}
	resp, err := m.R().
		SetContext(ctx).
		SetHeaderMultiValues(map[string][]string{
			UserIdHeaderKey: {req.UserID},
			OrgIdHeaderKey:  {req.OrgID},
		}).
		SetPathParam("id", strconv.FormatInt(req.MachineID, 10)).
		SetResult(&res).
		Post("/api/v1/node/del/{id}")
	if err != nil {
		logger.Errorf(ctx, "DelMachine fail machine req: %+v, err: %+v", *req, err)
		return code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "DelMachine fail machine req: %+v, http code: %+v", *req, resp.StatusCode())
		return code.RPCHttpCodeErr.WithMsgf("http code: %d", resp.StatusCode())
	}

	if res.Code != code.Success {
		logger.Errorf(ctx, "DelMachine code not success machine req: %+v, code: %+d", *req, res.Code)
		return code.RPCHttpCodeErr.WithMsgf("code: %d", res.Code)
	}

	return nil
}

func (m *MachineImpl) MachineStatus(ctx context.Context, req *model.MachineStatusReq) (*model.MachineStatusRes, error) {
	res := common.RespT[*model.MachineStatusRes]{}
	resp, err := m.R().
		SetContext(ctx).
		SetHeaderMultiValues(map[string][]string{
			UserIdHeaderKey: {req.UserID},
			OrgIdHeaderKey:  {req.OrgID},
		}).
		SetPathParam("id", strconv.FormatInt(req.MachineID, 10)).
		SetResult(&res).
		Get("/api/v1/node/{id}")
	if err != nil {
		logger.Errorf(ctx, "MachineStatus fail machine req: %+v, err: %+v", *req, err)
		return nil, code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "MachineStatus fail machine req: %+v, http code: %+v", *req, resp.StatusCode())
		return nil, code.RPCHttpCodeErr.WithMsgf("http code: %d", resp.StatusCode())
	}

	if res.Code != code.Success {
		logger.Errorf(ctx, "MachineStatus code not success machine req: %+v, code: %+d", *req, res.Code)
		return nil, code.RPCHttpCodeErr.WithMsgf("code: %d", res.Code)
	}

	return res.Data, nil
}

func (m *MachineImpl) StopMachine(ctx context.Context, req *model.StopMachineReq) error {
	req.CreatorID, _ = strconv.ParseUint(req.UserID, 10, 64)
	req.Device = "container"
	res := common.Resp{}
	resp, err := m.R().
		SetContext(ctx).
		SetPathParam("id", strconv.FormatInt(req.MachineID, 10)).
		SetHeaderMultiValues(map[string][]string{
			UserIdHeaderKey: {req.UserID},
			OrgIdHeaderKey:  {req.OrgID},
		}).
		SetBody(req).
		SetResult(&res).
		Post("/api/v1/node/stop/{id}")
	if err != nil {
		logger.Errorf(ctx, "StopMachine fail machine req: %+v, err: %+v", *req, err)
		return code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "StopMachine fail machine req: %+v, http code: %+v", *req, resp.StatusCode())
		return code.RPCHttpCodeErr.WithMsgf("http code: %d", resp.StatusCode())
	}

	if res.Code != code.Success {
		logger.Errorf(ctx, "StopMachine code not success machine id: %+v, code: %+d", *req, res.Code)
		return code.RPCHttpCodeErr.WithMsgf("code: %d", res.Code)
	}

	return nil
}

func (m *MachineImpl) CreateMachine(ctx context.Context, req *model.CreateMachineReq) (int64, error) {
	res := &common.RespT[*CreateNodeResp]{}
	resp, err := m.R().SetContext(ctx).
		SetBody(req).
		SetHeaderMultiValues(map[string][]string{
			UserIdHeaderKey: {req.UserID},
			OrgIdHeaderKey:  {req.OrgID},
		}).
		SetResult(&res).
		Post("/api/v1/node/add")
	if err != nil {
		logger.Errorf(ctx, "CreateMachine fail req: %+v, err: %+v", *req, err)
		return 0, code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "CreateMachine fail req: %+v, http code: %+v", *req, resp.StatusCode())
		return 0, code.RPCHttpCodeErr.WithMsgf("http code: %d", resp.StatusCode())
	}

	if res.Code != code.Success {
		if strings.HasPrefix(res.Error.Msg, "The maximum number of your nodes in the same project is 1") {
			return 0, code.MachineReachMaxNumCountErr
		}
		logger.Errorf(ctx, "CreateMachine code not success req: %+v, code: %+d, err: %+v", *req, res.Code, *res.Error)
		return 0, code.RPCHttpCodeErr.WithMsgf("code: %d", res.Code)
	}

	return res.Data.ID, nil
}

func (m *MachineImpl) JoinProject(ctx context.Context, req *model.JoninProjectReq) error {
	res := &common.Resp{}
	resp, err := m.R().SetContext(ctx).
		SetBody(req).
		SetHeaderMultiValues(map[string][]string{
			UserIdHeaderKey: {req.UserID},
			OrgIdHeaderKey:  {req.OrgID},
		}).
		SetResult(&res).
		Post("/api/v2/project/join")
	if err != nil {
		logger.Errorf(ctx, "JoinProject fail req: %+v, err: %+v", *req, err)
		return code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "JoinProject fail req: %+v, http code: %+v", *req, resp.StatusCode())
		return code.RPCHttpCodeErr.WithMsgf("http code: %d", resp.StatusCode())
	}

	if res.Code == code.Success || res.Code == 140112 {
		return nil
	}

	logger.Errorf(ctx, "JoinProject code not success req: %+v, code: %+d", *req, res.Code)
	return code.RPCHttpCodeErr.WithMsgf("code: %d", res.Code)
}

func (m *MachineImpl) RestartMachine(ctx context.Context, req *model.RestartMachineReq) error {
	res := &common.Resp{}
	resp, err := m.R().SetContext(ctx).
		SetBody(req).
		SetHeaderMultiValues(map[string][]string{
			UserIdHeaderKey: {req.UserID},
			OrgIdHeaderKey:  {req.OrgID},
		}).
		SetPathParam("id", strconv.FormatInt(req.MachineID, 10)).
		SetResult(&res).
		Post("/api/v1/node/restart/{id}")
	if err != nil {
		logger.Errorf(ctx, "RestartMachine fail req: %+v, err: %+v", *req, err)
		return code.RPCHttpErr.WithErr(err)
	}

	if resp.StatusCode() != http.StatusOK {
		logger.Errorf(ctx, "RestartMachine fail req: %+v, http code: %+v", *req, resp.StatusCode())
		return code.RPCHttpCodeErr.WithMsgf("http code: %d", resp.StatusCode())
	}

	if res.Code != code.Success {
		logger.Errorf(ctx, "RestartMachine code not success req: %+v, code: %+d", *req, res.Code)
		return code.RPCHttpCodeRespErr.WithMsgf("code: %d", res.Code)
	}
	return nil
}
