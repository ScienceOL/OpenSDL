package material

import (
	"bytes"
	"context"
	"encoding/json"
	"errors"
	"fmt"
	"net/http"

	"github.com/gin-gonic/gin"
	"github.com/gorilla/websocket"
	"github.com/olahol/melody"
	"github.com/scienceol/osdl/pkg/common"
	"github.com/scienceol/osdl/pkg/common/code"
	"github.com/scienceol/osdl/pkg/common/constant"
	"github.com/scienceol/osdl/pkg/common/uuid"
	"github.com/scienceol/osdl/pkg/core/material"
	impl "github.com/scienceol/osdl/pkg/core/material/material"
	"github.com/scienceol/osdl/pkg/middleware/auth"
	"github.com/scienceol/osdl/pkg/middleware/logger"
)

type Handle struct {
	mService material.Service
	wsClient *melody.Melody
}

func NewMaterialHandle(ctx context.Context) *Handle {
	wsClient := melody.New()
	wsClient.Config.MaxMessageSize = constant.MaxMessageSize
	mService := impl.NewMaterial(ctx, wsClient)

	h := &Handle{
		mService: mService,
		wsClient: wsClient,
	}

	h.initMaterialWebSocket()
	return h
}

func (m *Handle) CreateLabMaterial(ctx *gin.Context) {
	req := &material.GraphNodeReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse CreateLabMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	if err := m.mService.CreateMaterial(ctx, req); err != nil {
		logger.Errorf(ctx, "CreateMaterial err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}
	common.ReplyOk(ctx)
}

func (m *Handle) EdgeCreateMaterial(ctx *gin.Context) {
	req := &material.CreateMaterialReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse EdgeCreateMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.EdgeCreateMaterial(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) EdgeUpsertMaterial(ctx *gin.Context) {
	req := &material.UpsertMaterialReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse EdgeUpsertMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.EdgeUpsertMaterial(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) EdgeCreateEdge(ctx *gin.Context) {
	req := &material.CreateMaterialEdgeReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse EdgeCreateEdge param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	err := m.mService.EdgeCreateEdge(ctx, req)
	common.Reply(ctx, err)
}

func (m *Handle) SaveMaterial(ctx *gin.Context) {
	req := &material.SaveGrapReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse SaveMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	if err := m.mService.SaveMaterial(ctx, req); err != nil {
		logger.Errorf(ctx, "SaveMaterial err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}
	common.ReplyOk(ctx)
}

func (m *Handle) QueryMaterial(ctx *gin.Context) {
	req := &material.MaterialReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse QueryMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.LabMaterial(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) QueryMaterialByUUID(ctx *gin.Context) {
	req := &material.MaterialQueryReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse QueryMaterialByUUID param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.EdgeQueryMaterial(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) EdgeDownloadMaterial(ctx *gin.Context) {
	resp, err := m.mService.EdgeDownloadMaterial(ctx)
	common.Reply(ctx, err, resp)
}

func (m *Handle) BatchUpdateMaterial(ctx *gin.Context) {
	req := &material.UpdateMaterialReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse BatchUpdateMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	err := m.mService.BatchUpdateUniqueName(ctx, req)
	common.Reply(ctx, err)
}

func (m *Handle) ResourceList(ctx *gin.Context) {
	req := &material.ResourceReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse ResourceList param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.ResourceList(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) Actions(ctx *gin.Context) {
	req := &material.DeviceActionReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "parse Actions param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.DeviceAction(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) CreateMaterialEdge(ctx *gin.Context) {
	req := &material.GraphEdge{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		logger.Errorf(ctx, "parse CreateMaterialEdge param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	if err := m.mService.CreateEdge(ctx, req); err != nil {
		logger.Errorf(ctx, "CreateMaterialEdge err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}
	common.ReplyOk(ctx)
}

func (m *Handle) DownloadMaterial(ctx *gin.Context) {
	req := &material.DownloadMaterial{}
	if err := ctx.ShouldBindUri(req); err != nil {
		logger.Errorf(ctx, "parse DownloadMaterial param err: %+v", err.Error())
		common.ReplyErr(ctx, code.ParamErr, err.Error())
		return
	}
	resp, err := m.mService.DownloadMaterial(ctx, req)
	if err != nil {
		logger.Errorf(ctx, "DownloadMaterial err: %+v", err)
		common.ReplyErr(ctx, err)
		return
	}
	commonResp := &common.Resp{
		Code: code.Success,
		Data: resp,
	}
	data, err := json.Marshal(commonResp)
	if err != nil {
		logger.Errorf(ctx, "DownloadMaterial marshal err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithErr(err))
		return
	}
	ctx.Header("Cache-Control", "no-cache")
	ctx.Header("Content-Disposition", "attachment; filename=material_graph.json")
	ctx.Header("Content-Type", "application/json")
	ctx.Header("Pragma", "public")
	ctx.Header("Content-Length", fmt.Sprintf("%d", len(data)))
	reader := bytes.NewReader(data)
	ctx.DataFromReader(http.StatusOK, int64(len(data)), "application/json", reader, nil)
}

func (m *Handle) Template(ctx *gin.Context) {
	req := &material.TemplateReq{}
	if err := ctx.ShouldBindUri(req); err != nil {
		logger.Errorf(ctx, "MaterialTemplate err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithErr(err))
		return
	}
	resp, err := m.mService.GetMaterialTemplate(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) GetResourceNodeTemplate(ctx *gin.Context) {
	req := &material.AllTemplateReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		logger.Errorf(ctx, "GetResourceNodeTemplate err: %+v", err)
		common.ReplyErr(ctx, code.ParamErr.WithErr(err))
		return
	}
	resp, err := m.mService.GetResourceNodeTemplate(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) StartMachine(ctx *gin.Context) {
	req := &material.StartMachineReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	resp, err := m.mService.StartMachine(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) StopMachine(ctx *gin.Context) {
	req := &material.StopMachineReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	err := m.mService.StopMachine(ctx, req)
	common.Reply(ctx, err)
}

func (m *Handle) DeleteMachine(ctx *gin.Context) {
	req := &material.DelMachineReq{}
	if err := ctx.ShouldBindJSON(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	err := m.mService.DelMachine(ctx, req)
	common.Reply(ctx, err)
}

func (m *Handle) MachineStatus(ctx *gin.Context) {
	req := &material.MachineStatusReq{}
	if err := ctx.ShouldBindQuery(req); err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	resp, err := m.mService.MachineStatus(ctx, req)
	common.Reply(ctx, err, resp)
}

func (m *Handle) LabMaterial(ctx *gin.Context) {
	req := &material.LabWS{}
	var err error
	labUUIDStr := ctx.Param("lab_uuid")
	req.LabUUID, err = uuid.FromString(labUUIDStr)
	if err != nil {
		common.ReplyErr(ctx, code.ParamErr.WithMsg(err.Error()))
		return
	}
	userInfo := auth.GetCurrentUser(ctx)

	if err := m.wsClient.HandleRequestWithKeys(ctx.Writer, ctx.Request, map[string]any{
		auth.USERKEY: userInfo,
		"ctx":        ctx,
		"lab_uuid":   req.LabUUID,
	}); err != nil {
		logger.Errorf(ctx, "LabMaterial HandleRequestWithKeys err: %+v", err)
	}
}

func (m *Handle) initMaterialWebSocket() {
	m.wsClient.HandleClose(func(s *melody.Session, _ int, _ string) error {
		if ctx, ok := s.Get("ctx"); ok {
			logger.Infof(ctx.(context.Context), "material ws client close keys: %+v", s.Keys)
		}
		return nil
	})

	m.wsClient.HandleDisconnect(func(s *melody.Session) {
		if ctx, ok := s.Get("ctx"); ok {
			logger.Infof(ctx.(context.Context), "material ws client disconnected keys: %+v", s.Keys)
		}
	})

	m.wsClient.HandleError(func(s *melody.Session, err error) {
		if errors.Is(err, melody.ErrMessageBufferFull) {
			return
		}
		if closeErr, ok := err.(*websocket.CloseError); ok {
			if closeErr.Code == websocket.CloseGoingAway {
				return
			}
		}
		if ctx, ok := s.Get("ctx"); ok {
			logger.Errorf(ctx.(context.Context), "material ws error keys: %+v, err: %+v", s.Keys, err)
		}
	})

	m.wsClient.HandleConnect(func(s *melody.Session) {
		if ctx, ok := s.Get("ctx"); ok {
			logger.Infof(ctx.(context.Context), "material ws connect keys: %+v", s.Keys)
			if err := m.mService.OnWSConnect(ctx.(context.Context), s); err != nil {
				logger.Errorf(ctx.(context.Context), "material OnWSConnect err: %+v", err)
			}
		}
	})

	m.wsClient.HandleMessage(func(s *melody.Session, b []byte) {
		ctxI, ok := s.Get("ctx")
		if !ok {
			if err := s.CloseWithMsg([]byte("no ctx")); err != nil {
				logger.Errorf(context.Background(), "HandleMessage ctx not exist CloseWithMsg err: %+v", err)
			}
			return
		}
		if err := m.mService.OnWSMsg(ctxI.(*gin.Context), s, b); err != nil {
			logger.Errorf(ctxI.(*gin.Context), "material handle msg err: %+v", err)
		}
	})

	m.wsClient.HandleSentMessage(func(_ *melody.Session, _ []byte) {})
	m.wsClient.HandleSentMessageBinary(func(_ *melody.Session, _ []byte) {})
}
