# A stand-in for the editor's `unreal` module: just enough API for the pose maths to run
# without Unreal. The skeleton faces +Y in mesh space, like UE mannequin assets, so its right
# side is -X.
import math
class Vector:
    def __init__(s,x=0,y=0,z=0): s.x,s.y,s.z=x,y,z
class Quat:
    def __init__(s,x,y,z,w): s.x,s.y,s.z,s.w=x,y,z,w
class Rotator:
    def __init__(s,roll=0.0,pitch=0.0,yaw=0.0): s.roll,s.pitch,s.yaw=roll,pitch,yaw
    def quaternion(s):
        # yaw only is enough for these tests
        h=math.radians(s.yaw)/2; hp=math.radians(s.pitch)/2
        assert s.roll==0
        # yaw * pitch (Z then Y), as FRotator::Quaternion: positive pitch is nose up, Y = -sin(pitch/2)
        qy=(0,0,math.sin(h),math.cos(h)); qp=(0,-math.sin(hp),0,math.cos(hp))
        ax,ay,az,aw=qy; bx,by,bz,bw=qp
        return Quat(aw*bx+ax*bw+ay*bz-az*by, aw*by-ax*bz+ay*bw+az*bx, aw*bz+ax*by-ay*bx+az*bw, aw*bw-ax*bx-ay*by-az*bz)
class Transform:
    def __init__(s,t,q=(0,0,0,1)): s.translation=Vector(*t); s.rotation=Quat(*q); s.scale3d=Vector(1,1,1)
# Skeleton facing +Y in mesh space (like UE mannequin assets): right side is -X.
BONES=[("root",None,(0,0,0)),("pelvis","root",(0,0,95)),("spine_01","pelvis",(0,0,20)),("spine_03","spine_01",(0,0,25)),
 ("head","spine_03",(0,0,30)),("upperarm_r","spine_03",(-20,0,0)),("lowerarm_r","upperarm_r",(-25,0,0)),("hand_r","lowerarm_r",(-25,0,0)),
 ("upperarm_l","spine_03",(20,0,0)),("lowerarm_l","upperarm_l",(25,0,0)),("hand_l","lowerarm_l",(25,0,0)),
 ("thigh_r","pelvis",(-10,0,0)),("calf_r","thigh_r",(0,0,-45)),("foot_r","calf_r",(0,0,-45)),
 ("thigh_l","pelvis",(10,0,0)),("calf_l","thigh_l",(0,0,-45)),("foot_l","calf_l",(0,0,-45))]
class _Sock:
    def __init__(s,bone,loc): s.p={"bone_name":bone,"relative_location":Vector(*loc),"relative_rotation":Rotator(),"relative_scale":Vector(1,1,1)}
    def get_editor_property(s,k): return s.p[k]
class Bounds:
    def __init__(s,o,e): s.origin=Vector(*o); s.box_extent=Vector(*e)
class SkeletalMesh:
    def __init__(s,path): s.path=path
    def find_socket(s,n): return _Sock("hand_r",(0,0,0)) if n=="weapon_r" else None
    def get_path_name(s): return s.path
    def get_name(s): return s.path
class StaticMesh:
    def get_bounds(s): return Bounds((50,0,0),(50,3,1))
    def find_socket(s,n): return _Sock("",(0,0,0)) if n=="Grip" else None
    def get_path_name(s): return "/Game/Sword"
class SkeletalMeshComponent:
    def set_skeletal_mesh_asset(s,m): s.m=m
    def get_num_bones(s): return len(BONES)
    def get_bone_name(s,i): return BONES[i][0]
    def get_parent_bone(s,n): return dict((b[0],b[1]) for b in BONES)[n] or "None"
    def get_ref_pose_transform(s,i): return Transform(BONES[i][2])
def new_object(cls): return cls()
class AnimSequenceBase: pass
class Anim(AnimSequenceBase):
    def get_play_length(s): return 1.0
    def get_path_name(s): return "/Game/Swing"
class AnimationLibrary:
    @staticmethod
    def get_bone_pose_for_time(anim,bone,t,rm):
        loc=dict((b[0],b[2]) for b in BONES)[bone]
        if bone=="upperarm_r":  # swing the right arm forward (+Y) then across the body (+X)
            a=math.radians(180*t)  # rotate arm about Z
            h=a/2; return Transform(loc,(0,0,-math.sin(h),math.cos(h)))
        return Transform(loc)
# The whole pose at one time, as the editor evaluates it; built on the per-bone call above so a
# test that replaces that call changes both.
class AnimPoseSpaces: LOCAL="local"; WORLD="world"
class AnimPoseEvaluationOptions: pass
class _Pose:
    def __init__(s,anim,t): s.anim,s.t=anim,t
class AnimPoseExtensions:
    @staticmethod
    def get_anim_pose_at_time(anim,t,options): return _Pose(anim,t)
    @staticmethod
    def get_bone_names(pose): return [b[0] for b in BONES]
    @staticmethod
    def get_bone_pose(pose,bone,space):
        assert space==AnimPoseSpaces.LOCAL
        return AnimationLibrary.get_bone_pose_for_time(pose.anim,bone,pose.t,False)
ASSETS={"/Game/Manny":SkeletalMesh("/Game/Manny"),"/Game/Swing":Anim(),"/Game/Sword":StaticMesh()}
def load_asset(p): return ASSETS.get(p)
